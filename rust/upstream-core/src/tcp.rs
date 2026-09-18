//! Slice 2 fresh plain-TCP exchange and DNS stream framing.
//!
//! DNS-over-TCP carries every message as exactly a two-byte unsigned big-endian
//! payload length prefix followed by the payload bytes. This module owns the
//! transport-level framing and exact frame I/O only: the size gate before any
//! write, the two-byte prefix, full-write semantics, and exact prefix/body
//! reads that reassemble fragments into one message.
//!
//! [`exchange`] composes those helpers with exactly one fresh `TcpStream` per
//! call. It is deliberately independent of policy, fallback, pooling, reuse,
//! pipelining, and retry: there is no second framing implementation, no
//! connection is kept after the call, and connect, the full framed write, and
//! the exact prefix/body read all race the same absolute deadline against
//! caller cancellation and owner close.

use std::future::{Future, poll_fn};
use std::pin::Pin;
use std::task::Poll;
use std::time::Instant;

use mosdns_dns_core::{FrameMode, frame_response, inspect_response_header, validate_response};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;

use crate::{
    ExchangeControl, ExchangeResponse, PreparedExchange, RequestError, ResponseCommit,
    SideEffectState, Transport, UpstreamError,
};

/// The fixed width of the DNS-over-TCP payload length prefix.
const PREFIX_BYTES: usize = 2;

/// Runs one fresh plain-TCP exchange for a prepared request.
///
/// The caller has already validated the request, transport, owner lifecycle,
/// and the pre-connect outbound-size gate through
/// [`crate::Upstream::prepare_exchange`]. This primitive connects exactly one
/// new [`TcpStream`] to the configured numeric endpoint, writes exactly one
/// unchanged query frame, reads exactly one complete response frame, and
/// enforces QR, the original request ID, and full dns-core response validation
/// before the response-commit gate. Connect, the full framed write, and the
/// exact prefix/body read all race the same absolute deadline against owner
/// close and caller cancellation. The stream is owned by this call and is
/// dropped on every return path, so it is never pooled, reused, retried, or
/// left open.
pub(crate) async fn exchange(
    prepared: &PreparedExchange<'_>,
    commit: &ResponseCommit<'_>,
) -> Result<ExchangeResponse, UpstreamError> {
    debug_assert_eq!(prepared.endpoint().transport(), Transport::Tcp);

    // Before connect: owner close, caller cancellation, then absolute deadline.
    prepared.check_at(Instant::now(), SideEffectState::NotSent)?;

    let endpoint = prepared.endpoint().address();
    let request_id = prepared.request().request_id();
    let query = prepared.request().query();
    // One absolute deadline for the whole logical exchange: connect, the full
    // framed write, and the exact prefix/body read all observe this same
    // instant. No phase starts a fresh relative timeout or resets it.
    let deadline = prepared.context().deadline();

    // One fresh connection per exchange. A connect/setup failure has sent no
    // DNS payload, so it is `Connect` (`NotSent`). The connect itself races
    // owner close, caller cancellation, and the shared absolute deadline.
    let mut stream = race_io(prepared, SideEffectState::NotSent, deadline, async {
        TcpStream::connect(endpoint)
            .await
            .map_err(|_| UpstreamError::Connect)
    })
    .await?;

    // Connect is an async wake: re-apply the full control before the first
    // write. Nothing has been written yet, so this check is still `NotSent`.
    prepared.check_at(Instant::now(), SideEffectState::NotSent)?;

    // Exactly one unchanged query frame, with full-write semantics. The
    // outbound-size gate runs before the first write here and again before
    // connect in `prepare_exchange`, so an unframeable query never reaches the
    // socket. A control error during a potentially partial write is
    // conservatively `MaybeSent`.
    race_io(
        prepared,
        SideEffectState::MaybeSent,
        deadline,
        write_frame(&mut stream, query),
    )
    .await?;

    // Exactly one complete response frame: the two-byte prefix and only the
    // declared body, with stream fragments reassembled into one message. The
    // request frame was fully written, so a control error while waiting for
    // the prefix or body is `Sent`.
    let body = race_io(
        prepared,
        SideEffectState::Sent,
        deadline,
        read_frame(&mut stream),
    )
    .await?;

    // A response shorter than the header or with QR clear cannot be attributed
    // to this exchange as a response.
    let header = inspect_response_header(&body).map_err(|_| UpstreamError::MalformedResponse)?;
    // The accepted response must answer this request, preserving the caller's
    // original DNS ID.
    if header.id != request_id {
        return Err(UpstreamError::ResponseMismatch);
    }
    // Only a complete, dns-core-valid response may be returned. A complete TCP
    // frame is the authoritative response, not a UDP TC observation.
    if validate_response(&body).is_err() {
        return Err(UpstreamError::MalformedResponse);
    }

    // Validation is complete. The final control-aware commit is the single
    // linearization point against owner close, caller cancellation, and the
    // exchange's original absolute deadline, immediately before the owned
    // response. Owner close is checked first, then caller cancellation, then
    // the already-established deadline; a success can never be reversed by a
    // later close, cancellation, or deadline. The commit starts no timer and
    // resets no deadline.
    let caller_cancellation = prepared.context().cancellation();
    commit.before_commit().await;
    commit.commit_final(&caller_cancellation, deadline, SideEffectState::Sent)?;
    Ok(ExchangeResponse::new(
        body,
        request_id,
        header.id,
        Transport::Tcp,
        false,
    ))
}

/// Encodes one outbound DNS-over-TCP message.
///
/// The payload must be non-zero, because a zero-length DNS frame is malformed
/// and cannot be produced from an empty query; that case maps to
/// [`UpstreamError::InvalidRequest`] with the settled `NotSent` state. A
/// payload larger than `u16::MAX` cannot be represented by the two-byte prefix
/// and maps to [`UpstreamError::FrameTooLarge`] before any socket work.
///
/// The prefix encoding itself reuses `dns-core`'s frozen `Stream` framing
/// helper, so upstream-core does not add a second framing implementation.
pub(crate) fn encode_frame(payload: &[u8]) -> Result<Vec<u8>, UpstreamError> {
    if payload.is_empty() {
        return Err(UpstreamError::InvalidRequest(RequestError::Empty));
    }
    frame_response(payload, FrameMode::Stream).map_err(|_| UpstreamError::FrameTooLarge)
}

/// Writes exactly one framed message with full-write semantics.
///
/// The frame is fully encoded before the first write, so an unframeable
/// payload never touches the stream. The write loop retries partial writes: a
/// short write is progress, not a message boundary.
///
/// The reported `SideEffectState` preserves write progress, because that is the
/// only sound basis for allowing a replacement connection:
///
/// * **No byte accepted** — `Send(NotSent)`. The kernel never took any part of
///   the frame, so the query provably did not reach the peer and a fresh
///   connection cannot double-send. This is what makes the owners' single
///   replacement rule reachable at all.
/// * **Any byte accepted** — `Send(MaybeSent)`. The kernel may already have put
///   part of the frame on the wire, so a retry could duplicate the query.
pub(crate) async fn write_frame<W>(writer: &mut W, payload: &[u8]) -> Result<(), UpstreamError>
where
    W: AsyncWrite + Unpin,
{
    let frame = encode_frame(payload)?;
    write_all_bytes(writer, &frame)
        .await
        .map_err(|failure| UpstreamError::Send(failure.side_effect()))
}

/// Flushes every buffered byte of a framed message to the transport.
///
/// A plain `TcpStream` has no write buffer to drain, but a TLS stream does:
/// `poll_write` may accept a whole frame into the session's record buffer
/// without the bytes having reached the socket. A complete DoT send therefore
/// requires an explicit flush, and `write_frame` alone is not sufficient
/// evidence that the query reached the peer.
///
/// A flush failure is reported with the caller's `side_effect`, because the
/// bytes preceding it were already accepted by the writer; the caller decides
/// whether that is a conservative `MaybeSent` or an established `Sent`.
pub(crate) async fn flush_bytes<W>(
    writer: &mut W,
    side_effect: SideEffectState,
) -> Result<(), UpstreamError>
where
    W: AsyncWrite + Unpin,
{
    poll_fn(|cx| Pin::new(&mut *writer).poll_flush(cx))
        .await
        .map_err(|_| UpstreamError::Send(side_effect))
}

/// Reads exactly one framed DNS message, reassembling stream fragments.
///
/// The reader consumes exactly two prefix bytes and then exactly the declared
/// body length, so neither a stream fragment nor a following message is
/// treated as a boundary. A zero-length prefix is
/// [`UpstreamError::MalformedResponse`]; EOF before a complete prefix or body
/// is [`UpstreamError::TruncatedFrame`]; any other read failure is a
/// non-framing [`UpstreamError::Receive`] recorded as `Sent`, because the
/// request frame was already fully written before this read.
pub(crate) async fn read_frame<R>(reader: &mut R) -> Result<Vec<u8>, UpstreamError>
where
    R: AsyncRead + Unpin,
{
    let mut prefix = [0u8; PREFIX_BYTES];
    read_exact_bytes(reader, &mut prefix).await?;
    let length = u16::from_be_bytes(prefix);
    if length == 0 {
        return Err(UpstreamError::MalformedResponse);
    }
    let mut body = vec![0u8; usize::from(length)];
    read_exact_bytes(reader, &mut body).await?;
    Ok(body)
}

/// Races one transport I/O future against owner close, caller cancellation, and
/// the exchange's single absolute deadline.
///
/// The `biased` branch order is the contract: when more than one control is
/// ready on the same poll, owner shutdown wins and reports
/// [`UpstreamError::Closed`], caller cancellation is next and reports
/// [`UpstreamError::Cancelled`], and the absolute deadline reports
/// [`UpstreamError::DeadlineExceeded`]. `side_effect` is the caller's current
/// state and is recorded on a control error only; a ready I/O future is
/// returned unchanged.
///
/// `deadline` is the already-established absolute instant, so this helper
/// never starts a second relative timeout and never resets the deadline. It
/// creates no runtime, spawns no task, and owns no socket of its own.
///
/// The control is taken as an [`ExchangeControl`] rather than a whole prepared
/// exchange so the secure transports can reuse this exact race for their own
/// connect/handshake/write/flush/read phases.
///
/// The error type is generic over `From<UpstreamError>` so the secure path can
/// receive its typed control failures as `SecureError` while the plain
/// transports keep receiving `UpstreamError`.
pub(crate) async fn race_control<E, F, T>(
    control: &ExchangeControl,
    side_effect: SideEffectState,
    deadline: Instant,
    io: F,
) -> Result<T, E>
where
    E: From<UpstreamError>,
    F: Future<Output = Result<T, E>>,
{
    let owner = control.owner_cancellation();
    let caller = control.caller_cancellation();
    let owner_cancelled = owner.cancelled();
    let caller_cancelled = caller.cancelled();
    tokio::pin!(owner_cancelled);
    tokio::pin!(caller_cancelled);

    // One absolute deadline, converted once; every control shares this wait.
    let deadline_timer = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline));
    tokio::pin!(deadline_timer);

    tokio::pin!(io);

    tokio::select! {
        biased;
        () = &mut owner_cancelled => Err(UpstreamError::Closed(side_effect).into()),
        () = &mut caller_cancelled => Err(UpstreamError::Cancelled(side_effect).into()),
        () = &mut deadline_timer => Err(UpstreamError::DeadlineExceeded(side_effect).into()),
        result = &mut io => result,
    }
}

/// Races one transport I/O future for a prepared exchange.
///
/// A thin wrapper over [`race_control`] for the existing plain-TCP primitive.
pub(crate) async fn race_io<F, T>(
    prepared: &PreparedExchange<'_>,
    side_effect: SideEffectState,
    deadline: Instant,
    io: F,
) -> Result<T, UpstreamError>
where
    F: Future<Output = Result<T, UpstreamError>>,
{
    race_control(prepared.control(), side_effect, deadline, io).await
}

/// A partial-write failure that carries how much of the frame was accepted.
///
/// This is the minimum information the transport contract needs to decide
/// whether a replacement connection is safe, and it exists only on this private
/// path: the public error surface keeps its closed `SideEffectState` shape.
#[derive(Debug)]
struct WriteFailure {
    /// Bytes the writer accepted before failing. Zero means nothing crossed.
    accepted: usize,
}

impl WriteFailure {
    /// `NotSent` only when nothing at all was accepted.
    const fn side_effect(&self) -> SideEffectState {
        if self.accepted == 0 {
            SideEffectState::NotSent
        } else {
            SideEffectState::MaybeSent
        }
    }
}

/// Writes every byte of `buffer`, retrying partial writes until the whole slice
/// has been accepted.
///
/// Progress is tracked across the whole loop so a failure can report whether the
/// writer had already accepted anything: a zero-byte `poll_write` is a distinct
/// `WriteZero` failure rather than silent progress, and it is classified by the
/// bytes accepted *before* it.
async fn write_all_bytes<W>(writer: &mut W, mut buffer: &[u8]) -> Result<(), WriteFailure>
where
    W: AsyncWrite + Unpin,
{
    let total = buffer.len();
    while !buffer.is_empty() {
        let polled = poll_fn(|cx| Pin::new(&mut *writer).poll_write(cx, buffer)).await;
        let written = match polled {
            Ok(written) => written,
            Err(_) => {
                return Err(WriteFailure {
                    accepted: total - buffer.len(),
                });
            }
        };
        if written == 0 {
            return Err(WriteFailure {
                accepted: total - buffer.len(),
            });
        }
        buffer = &buffer[written..];
    }
    Ok(())
}

/// Reads exactly `buffer.len()` bytes, separating end-of-stream from ordinary
/// read failures so a truncation is never reported as a generic receive error.
async fn read_exact_bytes<R>(reader: &mut R, buffer: &mut [u8]) -> Result<(), UpstreamError>
where
    R: AsyncRead + Unpin,
{
    let mut filled = 0;
    while filled < buffer.len() {
        let polled = poll_fn(|cx| {
            let mut chunk = ReadBuf::new(&mut buffer[filled..]);
            match Pin::new(&mut *reader).poll_read(cx, &mut chunk) {
                Poll::Ready(Ok(())) => Poll::Ready(Ok(chunk.filled().len())),
                Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
                Poll::Pending => Poll::Pending,
            }
        })
        .await;
        match polled {
            // A zero-length read is end-of-stream: the frame ended early.
            Ok(0) => return Err(UpstreamError::TruncatedFrame),
            Ok(read) => filled += read,
            Err(error) => {
                return Err(if error.kind() == std::io::ErrorKind::UnexpectedEof {
                    UpstreamError::TruncatedFrame
                } else {
                    UpstreamError::Receive(SideEffectState::Sent)
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::io::{Read, Write};
    use std::net::{Ipv4Addr, SocketAddr, TcpListener};
    use std::pin::Pin;
    use std::sync::Arc;
    use std::task::{Context, Poll};
    use std::time::{Duration, Instant};

    use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
    use tokio::time::timeout;

    use crate::{
        CloseResult, CloseTransition, CommitPause, Endpoint, ExchangeContext, ExchangeRequest,
        LifecycleState, PreparedExchange, RequestError, SideEffectState, Transport,
        TransportCancellation, Upstream, UpstreamError,
    };

    /// Bounds every control race so a broken helper cannot hang the test.
    const TEST_TIMEOUT: Duration = Duration::from_secs(5);

    /// Runs one bounded current-thread runtime for a single framing test.
    ///
    /// Every test double below completes on the first poll, so there is no
    /// timer, socket, or timing dependency to sleep on.
    fn block_on<F: Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build current-thread test runtime")
            .block_on(future)
    }

    /// Deterministic reader that returns at most `max_chunk` bytes per poll.
    ///
    /// Returning `Poll::Ready` with a partial fill (and no wakeup) makes
    /// fragmentation repeatable without any timing or sleeps.
    struct ChunkedReader {
        data: Vec<u8>,
        cursor: usize,
        max_chunk: usize,
        polls: usize,
    }

    impl ChunkedReader {
        fn new(data: Vec<u8>, max_chunk: usize) -> Self {
            assert!(max_chunk >= 1, "test reader must make progress");
            Self {
                data,
                cursor: 0,
                max_chunk,
                polls: 0,
            }
        }

        /// The bytes not yet consumed by framing.
        fn remaining(&self) -> &[u8] {
            &self.data[self.cursor..]
        }
    }

    impl AsyncRead for ChunkedReader {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buffer: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            let this = &mut *self;
            let remaining = &this.data[this.cursor..];
            let take = remaining.len().min(this.max_chunk).min(buffer.remaining());
            if take > 0 {
                buffer.put_slice(&remaining[..take]);
                this.cursor += take;
                this.polls += 1;
            }
            // An empty remainder or a zero-length buffer leaves the read
            // buffer unfilled, which is the EOF signal `read_exact` observes.
            Poll::Ready(Ok(()))
        }
    }

    /// Reader that always fails with a non-EOF I/O error.
    struct FailingReader;

    impl AsyncRead for FailingReader {
        fn poll_read(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            _buffer: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionReset,
                "deliberate non-EOF read failure",
            )))
        }
    }

    /// Writer that accepts at most `max_chunk` bytes per poll and can be made
    /// to fail after a fixed number of accepted chunks.
    struct ShortWriter {
        written: Vec<u8>,
        max_chunk: usize,
        polls: usize,
        fail_after_chunks: Option<usize>,
    }

    impl ShortWriter {
        fn new(max_chunk: usize) -> Self {
            assert!(max_chunk >= 1, "test writer must make progress");
            Self {
                written: Vec::new(),
                max_chunk,
                polls: 0,
                fail_after_chunks: None,
            }
        }

        fn failing_after_chunks(mut self, chunks: usize) -> Self {
            self.fail_after_chunks = Some(chunks);
            self
        }
    }

    impl AsyncWrite for ShortWriter {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buffer: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            let this = &mut *self;
            if let Some(limit) = this.fail_after_chunks {
                if this.polls >= limit {
                    return Poll::Ready(Err(std::io::Error::new(
                        std::io::ErrorKind::BrokenPipe,
                        "deliberate short-write failure",
                    )));
                }
            }
            let take = buffer.len().min(this.max_chunk);
            this.written.extend_from_slice(&buffer[..take]);
            this.polls += 1;
            Poll::Ready(Ok(take))
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    /// A writer that accepts nothing at all, modelling a `WriteZero` result.
    struct ZeroWriter;

    impl AsyncWrite for ZeroWriter {
        fn poll_write(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            _buffer: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            Poll::Ready(Ok(0))
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[test]
    fn encode_frame_prefixes_payload_length_big_endian() {
        let payload: Vec<u8> = (0..300u32).map(|index| (index % 251) as u8).collect();
        let frame = super::encode_frame(&payload).expect("non-zero in-range payload encodes");

        assert_eq!(frame.len(), payload.len() + 2);
        assert_eq!(
            &frame[..2],
            &300u16.to_be_bytes(),
            "prefix is the payload length in big-endian order"
        );
        assert_eq!(&frame[2..], payload.as_slice());
    }

    #[test]
    fn encode_frame_accepts_max_u16_payload() {
        let payload = vec![0x7Au8; usize::from(u16::MAX)];
        let frame =
            super::encode_frame(&payload).expect("the largest representable payload encodes");

        assert_eq!(&frame[..2], &[0xFF, 0xFF]);
        assert_eq!(frame.len(), usize::from(u16::MAX) + 2);
    }

    #[test]
    fn encode_frame_rejects_empty_payload_as_invalid_request() {
        let error = super::encode_frame(&[]).expect_err("a zero-length payload cannot be framed");

        assert_eq!(error, UpstreamError::InvalidRequest(RequestError::Empty));
        assert_eq!(error.side_effect(), SideEffectState::NotSent);
    }

    #[test]
    fn encode_frame_rejects_payload_over_u16_max() {
        let payload = vec![0u8; usize::from(u16::MAX) + 1];
        let error = super::encode_frame(&payload).expect_err("oversized payload is rejected");

        assert_eq!(error, UpstreamError::FrameTooLarge);
        assert_eq!(error.side_effect(), SideEffectState::NotSent);
    }

    #[test]
    fn write_frame_emits_complete_frame_despite_short_writes() {
        block_on(async {
            let payload = vec![0xABu8; 9];
            let expected = super::encode_frame(&payload).expect("payload encodes");
            let mut writer = ShortWriter::new(1);

            super::write_frame(&mut writer, &payload)
                .await
                .expect("full write succeeds across short writes");

            assert_eq!(writer.written, expected);
            assert!(
                writer.polls > 1,
                "partial writes are progress, not message boundaries"
            );
        });
    }

    #[test]
    fn write_frame_rejects_oversize_before_writing_anything() {
        block_on(async {
            let payload = vec![0u8; usize::from(u16::MAX) + 1];
            let mut writer = ShortWriter::new(4);

            let error = super::write_frame(&mut writer, &payload)
                .await
                .expect_err("oversized payload never reaches the writer");

            assert_eq!(error, UpstreamError::FrameTooLarge);
            assert!(writer.written.is_empty(), "nothing may be written");
            assert_eq!(writer.polls, 0, "the size gate precedes every write");
        });
    }

    #[test]
    fn write_frame_rejects_empty_payload_before_writing_anything() {
        block_on(async {
            let mut writer = ShortWriter::new(4);

            let error = super::write_frame(&mut writer, &[])
                .await
                .expect_err("an empty payload never reaches the writer");

            assert_eq!(error, UpstreamError::InvalidRequest(RequestError::Empty));
            assert!(writer.written.is_empty(), "nothing may be written");
            assert_eq!(writer.polls, 0);
        });
    }

    #[test]
    fn write_frame_reports_partial_write_as_send_with_uncertain_state() {
        block_on(async {
            let payload = vec![0xCDu8; 16];
            let mut writer = ShortWriter::new(2).failing_after_chunks(2);

            let error = super::write_frame(&mut writer, &payload)
                .await
                .expect_err("a write failure after progress is reported");

            assert_eq!(error, UpstreamError::Send(SideEffectState::MaybeSent));
            assert!(
                !writer.written.is_empty(),
                "some bytes may have crossed the network"
            );
            assert!(
                writer.written.len() < payload.len() + 2,
                "the failure happened before the full frame was written"
            );
        });
    }

    /// A write that fails **before accepting any byte** must be `NotSent`.
    ///
    /// This is what makes the owners' single-replacement rule reachable: the
    /// connection was never written to, so a fresh one provably cannot
    /// double-send the query. Before this was tracked, every write failure
    /// reported `MaybeSent` and the replacement path was dead code.
    #[test]
    fn write_frame_reports_zero_progress_as_send_with_not_sent_state() {
        block_on(async {
            let payload = vec![0xABu8; 12];
            let mut writer = ShortWriter::new(4).failing_after_chunks(0);

            let error = super::write_frame(&mut writer, &payload)
                .await
                .expect_err("a write failure before any byte is reported");

            assert_eq!(
                error,
                UpstreamError::Send(SideEffectState::NotSent),
                "no byte was accepted, so the query provably did not reach the peer"
            );
            assert!(
                writer.written.is_empty(),
                "the writer must not have accepted part of the frame"
            );
        });
    }

    /// A `WriteZero` failure before any byte is also `NotSent`.
    ///
    /// A writer that accepts nothing is the same evidence as an outright error:
    /// no part of the frame left this process.
    #[test]
    fn write_frame_reports_zero_byte_write_as_send_with_not_sent_state() {
        block_on(async {
            let payload = vec![0x11u8; 8];
            let mut writer = ZeroWriter;

            let error = super::write_frame(&mut writer, &payload)
                .await
                .expect_err("a zero-byte write is a failure, not progress");

            assert_eq!(error, UpstreamError::Send(SideEffectState::NotSent));
        });
    }

    #[test]
    fn read_frame_reassembles_fragmented_reads_and_consumes_only_the_body() {
        block_on(async {
            let payload = vec![0x5Au8; 7];
            let trailing = vec![0xEEu8; 3];
            let mut wire = super::encode_frame(&payload).expect("payload encodes");
            wire.extend_from_slice(&trailing);
            // One byte per poll forces the prefix and body across many reads.
            let mut reader = ChunkedReader::new(wire, 1);

            let body = super::read_frame(&mut reader).await.expect("frame reads");

            assert_eq!(body, payload);
            assert!(reader.polls > 1, "fragments must be reassembled");
            assert_eq!(
                reader.remaining(),
                trailing.as_slice(),
                "only the declared body is consumed"
            );
        });
    }

    #[test]
    fn read_frame_returns_two_sequential_frames_as_separate_messages() {
        block_on(async {
            let first = vec![0x01u8; 3];
            let second = vec![0x02u8; 5];
            let mut wire = super::encode_frame(&first).expect("first frame encodes");
            wire.extend_from_slice(&super::encode_frame(&second).expect("second frame encodes"));
            let mut reader = ChunkedReader::new(wire, 2);

            assert_eq!(
                super::read_frame(&mut reader).await.expect("first frame"),
                first
            );
            assert_eq!(
                super::read_frame(&mut reader).await.expect("second frame"),
                second
            );
        });
    }

    #[test]
    fn read_frame_rejects_zero_length_prefix_as_malformed_before_body_read() {
        block_on(async {
            let mut reader = ChunkedReader::new(vec![0x00, 0x00, 0xAA], 1);

            let error = super::read_frame(&mut reader)
                .await
                .expect_err("a zero-length frame is malformed");

            assert_eq!(error, UpstreamError::MalformedResponse);
            assert_eq!(
                reader.remaining(),
                &[0xAA],
                "the zero-length prefix is rejected before any body read"
            );
        });
    }

    #[test]
    fn read_frame_eof_before_a_full_prefix_is_truncated() {
        block_on(async {
            let mut empty = ChunkedReader::new(Vec::new(), 1);
            assert_eq!(
                super::read_frame(&mut empty)
                    .await
                    .expect_err("no prefix bytes is truncated"),
                UpstreamError::TruncatedFrame
            );

            let mut one_byte = ChunkedReader::new(vec![0x00], 1);
            assert_eq!(
                super::read_frame(&mut one_byte)
                    .await
                    .expect_err("half a prefix is truncated"),
                UpstreamError::TruncatedFrame
            );
        });
    }

    #[test]
    fn read_frame_eof_during_the_body_is_truncated() {
        block_on(async {
            // The prefix declares eight body bytes but only three arrive.
            let mut reader = ChunkedReader::new(vec![0x00, 0x08, 0x01, 0x02, 0x03], 1);

            assert_eq!(
                super::read_frame(&mut reader)
                    .await
                    .expect_err("a partial body is truncated"),
                UpstreamError::TruncatedFrame
            );
        });
    }

    #[test]
    fn read_frame_maps_non_eof_read_failure_to_receive_with_sent_state() {
        block_on(async {
            let mut reader = FailingReader;

            let error = super::read_frame(&mut reader)
                .await
                .expect_err("a non-EOF read failure is not a truncation");

            assert_eq!(error, UpstreamError::Receive(SideEffectState::Sent));
        });
    }

    /// A valid query with a caller-chosen ID. Control races never open a
    /// socket, so this only has to satisfy the request boundary.
    fn query_wire(id: u16) -> Vec<u8> {
        let mut wire = Vec::new();
        wire.extend_from_slice(&id.to_be_bytes());
        wire.extend_from_slice(&[0x01, 0x00]); // RD=1, QR=0, opcode QUERY
        wire.extend_from_slice(&1u16.to_be_bytes()); // QDCOUNT
        wire.extend_from_slice(&0u16.to_be_bytes()); // ANCOUNT
        wire.extend_from_slice(&0u16.to_be_bytes()); // NSCOUNT
        wire.extend_from_slice(&0u16.to_be_bytes()); // ARCOUNT
        wire.extend_from_slice(&[0x07, b'e', b'x', b'a', b'm', b'p', b'l', b'e']);
        wire.extend_from_slice(&[0x03, b'o', b'r', b'g', 0x00]);
        wire.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]); // A IN
        wire
    }

    /// A numeric TCP endpoint; the control races below never dial it.
    fn tcp_endpoint() -> Endpoint {
        Endpoint::new(SocketAddr::from((Ipv4Addr::LOCALHOST, 1)), Transport::Tcp)
            .expect("numeric tcp endpoint")
    }

    /// A numeric TCP endpoint for a bound loopback port.
    fn tcp_endpoint_at(address: SocketAddr) -> Endpoint {
        Endpoint::new(address, Transport::Tcp).expect("numeric tcp endpoint")
    }

    /// Prepares one exchange borrowing `query` while the owner is still open.
    fn prepare<'q>(
        upstream: &Upstream,
        query: &'q [u8],
        context: ExchangeContext,
    ) -> PreparedExchange<'q> {
        let request = ExchangeRequest::new(query).expect("valid query");
        upstream
            .prepare_exchange(request, context)
            .expect("an open owner prepares the exchange")
    }

    /// A complete, dns-core-valid response with a one-byte answer marker.
    fn response_wire(id: u16, marker: u8) -> Vec<u8> {
        let mut wire = Vec::new();
        wire.extend_from_slice(&id.to_be_bytes());
        wire.extend_from_slice(&[0x81, 0x80]); // QR=1, RD=1, RA=1
        wire.extend_from_slice(&1u16.to_be_bytes()); // QDCOUNT
        wire.extend_from_slice(&1u16.to_be_bytes()); // ANCOUNT
        wire.extend_from_slice(&0u16.to_be_bytes()); // NSCOUNT
        wire.extend_from_slice(&0u16.to_be_bytes()); // ARCOUNT
        wire.extend_from_slice(&[0x07, b'e', b'x', b'a', b'm', b'p', b'l', b'e']);
        wire.extend_from_slice(&[0x03, b'o', b'r', b'g', 0x00]);
        wire.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]); // A IN
        wire.extend_from_slice(&[0xc0, 0x0c, 0x00, 0x01, 0x00, 0x01]); // owner ptr, A IN
        wire.extend_from_slice(&60u32.to_be_bytes());
        wire.extend_from_slice(&[0x00, 0x04, 192, 0, 2, marker]);
        wire
    }

    /// The shared far-future context used by the commit-gate tests whose
    /// control is not the deadline.
    fn open_context() -> ExchangeContext {
        ExchangeContext::new(
            Instant::now() + Duration::from_secs(30),
            TransportCancellation::new(),
        )
    }

    /// Accepts at most one connection within `budget`, or reports that none
    /// arrived.
    ///
    /// The listener is non-blocking so a regression that never opens a fresh
    /// connection fails the test instead of hanging the server thread.
    fn accept_within(listener: &TcpListener, budget: Duration) -> Option<std::net::TcpStream> {
        listener
            .set_nonblocking(true)
            .expect("switch listener to non-blocking");
        let deadline = Instant::now() + budget;
        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    stream
                        .set_nonblocking(false)
                        .expect("accepted stream is blocking");
                    return Some(stream);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return None;
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(error) => panic!("accept failed: {error}"),
            }
        }
    }

    /// One-connection loopback server: reads exactly one framed query, writes
    /// exactly one complete framed response, then drops the stream.
    ///
    /// The server runs on a plain OS thread so the bounded current-thread test
    /// runtime only has to poll the exchange under test.
    fn reply_server(reply: Vec<u8>) -> (SocketAddr, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind tcp loopback");
        let address = listener.local_addr().expect("local address");
        let handle = std::thread::spawn(move || {
            let mut stream =
                accept_within(&listener, TEST_TIMEOUT).expect("exactly one fresh connection");
            let mut prefix = [0u8; super::PREFIX_BYTES];
            stream.read_exact(&mut prefix).expect("read query prefix");
            let length = usize::from(u16::from_be_bytes(prefix));
            let mut body = vec![0u8; length];
            stream.read_exact(&mut body).expect("read query body");
            let length = u16::try_from(reply.len()).expect("reply body fits the u16 prefix");
            let mut frame = Vec::with_capacity(reply.len() + super::PREFIX_BYTES);
            frame.extend_from_slice(&length.to_be_bytes());
            frame.extend_from_slice(&reply);
            stream.write_all(&frame).expect("write response frame");
            stream.flush().expect("flush response frame");
        });
        (address, handle)
    }

    /// Installs the deterministic pre-commit pause seam and returns the test's
    /// handle to it.
    fn install_pause(upstream: &Upstream) -> Arc<CommitPause> {
        let pause = Arc::new(CommitPause::new());
        upstream.install_commit_pause(Arc::clone(&pause));
        pause
    }

    /// Spawns one full `Upstream::exchange` on the current-thread runtime.
    fn spawn_exchange(
        upstream: &Arc<Upstream>,
        query: Vec<u8>,
        context: ExchangeContext,
    ) -> tokio::task::JoinHandle<Result<crate::ExchangeResponse, UpstreamError>> {
        let upstream = Arc::clone(upstream);
        tokio::spawn(async move {
            let request = ExchangeRequest::new(&query).expect("valid query");
            upstream.exchange(request, context).await
        })
    }

    /// An I/O future that never becomes ready on its own.
    ///
    /// Every control test uses it to prove that owner close, caller
    /// cancellation, or the absolute deadline terminates the wait, never the
    /// I/O completing.
    struct PendingIo;

    impl Future for PendingIo {
        type Output = Result<u32, UpstreamError>;

        fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
            Poll::Pending
        }
    }

    #[test]
    fn race_io_owner_cancellation_wins_when_every_control_is_ready() {
        block_on(async {
            let upstream = Upstream::new(tcp_endpoint());
            let query = query_wire(0x7001);
            let caller = TransportCancellation::new();
            // The prepared context deadline is far in the future, but the
            // explicitly passed absolute deadline is already past, so owner
            // close, caller cancellation, and the deadline are all ready on the
            // first poll.
            let context =
                ExchangeContext::new(Instant::now() + Duration::from_secs(30), caller.clone());
            let prepared = prepare(&upstream, &query, context);

            assert_eq!(upstream.begin_close(), CloseTransition::BeganClosing);
            caller.cancel();
            let past = Instant::now() - Duration::from_secs(1);

            let outcome = timeout(
                TEST_TIMEOUT,
                super::race_io(&prepared, SideEffectState::Sent, past, PendingIo),
            )
            .await
            .expect("owner close interrupts the pending I/O immediately");

            assert_eq!(outcome, Err(UpstreamError::Closed(SideEffectState::Sent)));
        });
    }

    #[test]
    fn race_io_caller_cancellation_wins_over_the_deadline() {
        block_on(async {
            let upstream = Upstream::new(tcp_endpoint());
            let query = query_wire(0x7002);
            let caller = TransportCancellation::new();
            let context =
                ExchangeContext::new(Instant::now() + Duration::from_secs(30), caller.clone());
            let prepared = prepare(&upstream, &query, context);

            // The owner stays open, so caller cancellation and the past
            // absolute deadline are both ready and precedence decides.
            caller.cancel();
            let past = Instant::now() - Duration::from_secs(1);

            let outcome = timeout(
                TEST_TIMEOUT,
                super::race_io(&prepared, SideEffectState::MaybeSent, past, PendingIo),
            )
            .await
            .expect("caller cancellation interrupts the pending I/O immediately");

            assert_eq!(
                outcome,
                Err(UpstreamError::Cancelled(SideEffectState::MaybeSent))
            );
        });
    }

    #[test]
    fn race_io_uses_the_passed_absolute_deadline_without_resetting_it() {
        block_on(async {
            let upstream = Upstream::new(tcp_endpoint());
            let query = query_wire(0x7003);
            // The prepared context deadline is 30 seconds out; the helper must
            // honor the separately passed absolute deadline instead of starting
            // a fresh relative timeout or falling back to the context deadline.
            let context = ExchangeContext::new(
                Instant::now() + Duration::from_secs(30),
                TransportCancellation::new(),
            );
            let prepared = prepare(&upstream, &query, context);

            let past = Instant::now() - Duration::from_secs(1);

            let outcome = timeout(
                TEST_TIMEOUT,
                super::race_io(&prepared, SideEffectState::NotSent, past, PendingIo),
            )
            .await
            .expect("the past absolute deadline resolves immediately, not after 30s");

            assert_eq!(
                outcome,
                Err(UpstreamError::DeadlineExceeded(SideEffectState::NotSent))
            );
        });
    }

    #[test]
    fn race_io_returns_the_io_result_unchanged_when_no_control_is_ready() {
        block_on(async {
            let upstream = Upstream::new(tcp_endpoint());
            let query = query_wire(0x7004);
            let context = ExchangeContext::new(
                Instant::now() + Duration::from_secs(30),
                TransportCancellation::new(),
            );
            let prepared = prepare(&upstream, &query, context);
            let deadline = Instant::now() + Duration::from_secs(30);

            let ready = async { Ok::<_, UpstreamError>(0xABCD_u32) };
            let outcome = timeout(
                TEST_TIMEOUT,
                super::race_io(&prepared, SideEffectState::NotSent, deadline, ready),
            )
            .await
            .expect("ready I/O completes without waiting for a control");
            assert_eq!(outcome, Ok(0xABCD));

            // An I/O error is returned verbatim with its own typed category and
            // side-effect state rather than being relabelled as a timeout.
            let failing = async { Err::<u32, _>(UpstreamError::Receive(SideEffectState::Sent)) };
            let outcome = timeout(
                TEST_TIMEOUT,
                super::race_io(&prepared, SideEffectState::Sent, deadline, failing),
            )
            .await
            .expect("failing I/O completes without waiting for a control");
            assert_eq!(outcome, Err(UpstreamError::Receive(SideEffectState::Sent)));
        });
    }

    // -----------------------------------------------------------------------
    // Final control-aware response commit gate
    //
    // Each test drives the real `tcp::exchange` path against one bounded
    // loopback connection and parks it on the shared `CommitPause` seam after
    // the complete frame has been read, header/ID checked, and dns-core
    // validated. The test then makes exactly one control effective before
    // releasing the gate, so the outcome is decided by control precedence and
    // never by a sleep-only race guess.
    // -----------------------------------------------------------------------

    #[test]
    fn owner_close_at_the_final_commit_gate_is_closed_with_sent() {
        block_on(async {
            let id = 0x7101;
            let (address, server) = reply_server(response_wire(id, 11));
            let upstream = Arc::new(Upstream::new(tcp_endpoint_at(address)));
            let pause = install_pause(&upstream);
            let exchange = spawn_exchange(&upstream, query_wire(id), open_context());

            timeout(TEST_TIMEOUT, pause.arrived())
                .await
                .expect("exchange reaches the final commit gate");
            assert_eq!(upstream.in_flight_exchanges(), 1);

            // Owner close reaches the shared gate first while the exchange is
            // parked with a fully read and validated response.
            assert_eq!(upstream.begin_close(), CloseTransition::BeganClosing);
            assert_eq!(upstream.lifecycle_state(), LifecycleState::Closing);

            pause.release();
            let outcome = timeout(TEST_TIMEOUT, exchange)
                .await
                .expect("exchange bounded")
                .expect("exchange joined");
            assert_eq!(
                outcome
                    .err()
                    .expect("owner close wins the final commit gate"),
                UpstreamError::Closed(SideEffectState::Sent)
            );

            assert_eq!(upstream.close().await, CloseResult::Closed);
            assert_eq!(upstream.lifecycle_state(), LifecycleState::Closed);
            assert_eq!(upstream.in_flight_exchanges(), 0);
            server.join().expect("server thread joined");
        });
    }

    #[test]
    fn caller_cancellation_at_the_final_commit_gate_is_cancelled_with_sent() {
        block_on(async {
            let id = 0x7102;
            let (address, server) = reply_server(response_wire(id, 12));
            let upstream = Arc::new(Upstream::new(tcp_endpoint_at(address)));
            let pause = install_pause(&upstream);
            let cancellation = TransportCancellation::new();
            // The deadline stays far in the future so only caller cancellation
            // can win the gate.
            let context = ExchangeContext::new(
                Instant::now() + Duration::from_secs(30),
                cancellation.clone(),
            );
            let exchange = spawn_exchange(&upstream, query_wire(id), context);

            timeout(TEST_TIMEOUT, pause.arrived())
                .await
                .expect("exchange reaches the final commit gate");
            // The complete frame was read and validated; cancellation becomes
            // effective only now, after validation and before the commit.
            cancellation.cancel();
            pause.release();

            let outcome = timeout(TEST_TIMEOUT, exchange)
                .await
                .expect("exchange bounded")
                .expect("exchange joined");
            assert_eq!(
                outcome
                    .err()
                    .expect("caller cancellation wins the final commit gate"),
                UpstreamError::Cancelled(SideEffectState::Sent)
            );
            assert_eq!(upstream.in_flight_exchanges(), 0);
            server.join().expect("server thread joined");
        });
    }

    #[test]
    fn absolute_deadline_at_the_final_commit_gate_is_deadline_exceeded_with_sent() {
        block_on(async {
            let id = 0x7103;
            let (address, server) = reply_server(response_wire(id, 13));
            let upstream = Arc::new(Upstream::new(tcp_endpoint_at(address)));
            let pause = install_pause(&upstream);
            // The exact original absolute deadline for the whole exchange.
            // Loopback connect, write, and read complete far below it, so the
            // deadline can only become effective after validation, while the
            // exchange is parked at the final commit gate.
            let deadline = Instant::now() + Duration::from_millis(500);
            let context = ExchangeContext::new(deadline, TransportCancellation::new());
            let exchange = spawn_exchange(&upstream, query_wire(id), context);

            timeout(TEST_TIMEOUT, pause.arrived())
                .await
                .expect("exchange reaches the final commit gate before the deadline");
            // Wait for the exact original deadline instant, then release the
            // gate; no arbitrary sleep stands in for the deadline.
            tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
            pause.release();

            let outcome = timeout(TEST_TIMEOUT, exchange)
                .await
                .expect("exchange bounded")
                .expect("exchange joined");
            assert_eq!(
                outcome
                    .err()
                    .expect("the original absolute deadline wins the final commit gate"),
                UpstreamError::DeadlineExceeded(SideEffectState::Sent)
            );
            assert_eq!(upstream.in_flight_exchanges(), 0);
            server.join().expect("server thread joined");
        });
    }

    #[test]
    fn response_commit_at_the_final_gate_before_owner_close_is_not_reversed() {
        block_on(async {
            let id = 0x7104;
            let expected = response_wire(id, 14);
            let (address, server) = reply_server(expected.clone());
            let upstream = Arc::new(Upstream::new(tcp_endpoint_at(address)));
            let pause = install_pause(&upstream);
            let exchange = spawn_exchange(&upstream, query_wire(id), open_context());

            timeout(TEST_TIMEOUT, pause.arrived())
                .await
                .expect("exchange reaches the final commit gate");
            assert_eq!(upstream.lifecycle_state(), LifecycleState::Open);

            // The commit wins while the owner is Open and no control is
            // effective; the complete validated response is returned.
            pause.release();
            let response = timeout(TEST_TIMEOUT, exchange)
                .await
                .expect("exchange bounded")
                .expect("exchange joined")
                .expect("the response commits while the owner is Open and quiet");
            assert_eq!(response.transport(), Transport::Tcp);
            assert_eq!(response.response_id(), id);
            assert_eq!(response.wire(), expected.as_slice());
            assert!(!response.truncated());

            // A close that starts afterwards cannot reverse the committed
            // response and still drains the released registration to Closed.
            assert_eq!(upstream.close().await, CloseResult::Closed);
            assert_eq!(upstream.lifecycle_state(), LifecycleState::Closed);
            assert_eq!(upstream.in_flight_exchanges(), 0);
            server.join().expect("server thread joined");
        });
    }
}
