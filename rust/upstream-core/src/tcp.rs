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
//! connection is kept after the call, and deadline/cancellation racing is not
//! part of this step.

use std::future::poll_fn;
use std::pin::Pin;
use std::task::Poll;
use std::time::Instant;

use mosdns_dns_core::{FrameMode, frame_response, inspect_response_header, validate_response};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;

use crate::{
    ExchangeResponse, PreparedExchange, RequestError, ResponseCommit, SideEffectState, Transport,
    UpstreamError,
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
/// before the response-commit gate. The stream is owned by this call and is
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

    // One fresh connection per exchange. A connect/setup failure has sent no
    // DNS payload, so it is `Connect` (`NotSent`).
    let mut stream = TcpStream::connect(endpoint)
        .await
        .map_err(|_| UpstreamError::Connect)?;

    // Exactly one unchanged query frame, with full-write semantics. The
    // outbound-size gate runs before the first write here and again before
    // connect in `prepare_exchange`, so an unframeable query never reaches the
    // socket.
    write_frame(&mut stream, query).await?;

    // Exactly one complete response frame: the two-byte prefix and only the
    // declared body, with stream fragments reassembled into one message.
    let body = read_frame(&mut stream).await?;

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

    // Validation is complete; the commit gate is the single linearization
    // point against owner close, immediately before the owned response.
    commit.before_commit().await;
    commit.commit()?;
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
/// short write is progress, not a message boundary. A write failure after the
/// frame has begun maps to [`UpstreamError::Send`] with the conservative
/// `MaybeSent` state, because the kernel may already have accepted part of the
/// frame.
pub(crate) async fn write_frame<W>(writer: &mut W, payload: &[u8]) -> Result<(), UpstreamError>
where
    W: AsyncWrite + Unpin,
{
    let frame = encode_frame(payload)?;
    write_all_bytes(writer, &frame)
        .await
        .map_err(|_| UpstreamError::Send(SideEffectState::MaybeSent))
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

/// Writes every byte of `buffer`, retrying partial writes until the whole slice
/// has been accepted. A zero-byte write is a distinct `WriteZero` failure
/// rather than silent progress.
async fn write_all_bytes<W>(writer: &mut W, mut buffer: &[u8]) -> std::io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    while !buffer.is_empty() {
        let written = poll_fn(|cx| Pin::new(&mut *writer).poll_write(cx, buffer)).await?;
        if written == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WriteZero,
                "framed write made no progress",
            ));
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
    use std::pin::Pin;
    use std::task::{Context, Poll};

    use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

    use crate::{RequestError, SideEffectState, UpstreamError};

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
}
