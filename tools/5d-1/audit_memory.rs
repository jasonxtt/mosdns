// Diagnostic-only include inside observer.rs in the disposable BASE source.
// Call audit_memory_report(&state) in audit_snapshot before cloning records.
// This measures the original owned capacities, not the compacted clones.
fn audit_memory_report(state: &ObserverState) {
    use std::collections::BTreeSet;
    use std::mem::{size_of, size_of_val};

    fn optional(value: &Option<String>) -> usize {
        value.as_ref().map_or(0, String::capacity)
    }
    let mut fields = BTreeMap::<&str, usize>::new();
    let mut seen = BTreeSet::new();
    for record in state.audit_records.iter().chain(&state.slowest_records) {
        if !seen.insert(Arc::as_ptr(record)) {
            continue;
        }
        let record = record.as_ref();
        *fields.entry("record_inline_and_arc").or_default() += size_of_val(record) + 16;
        *fields.entry("qname").or_default() += record.qname.capacity();
        *fields.entry("trace_id").or_default() += record.trace_id.capacity();
        for (name, value) in [
            ("final_sequence", &record.final_sequence),
            ("matched_group", &record.matched_group),
            ("domain_set", &record.domain_set),
            ("effective_tag", &record.effective_tag),
            ("matched_rule_source", &record.matched_rule_source),
            ("final_upstream", &record.final_upstream),
            ("upstream_targets", &record.upstream_targets),
            ("selected_upstream", &record.selected_upstream),
        ] {
            *fields.entry(name).or_default() += optional(value);
        }
        *fields.entry("response_source").or_default() += match &record.response {
            ResponseState::Dns { source: ResponseSource::Upstream(value), .. } => value.capacity(),
            _ => 0,
        };
        let response = &record.response_details;
        *fields.entry("answer_vec").or_default() += response.answers.capacity() * size_of::<AuditAnswer>();
        *fields.entry("answer_data").or_default() += response.answers.iter().map(|answer| answer.data.capacity()).sum::<usize>();
        *fields.entry("answer_decode_error").or_default() += optional(&response.answer_decode_error);
        *fields.entry("upstream_attempts").or_default() += record.upstream_attempts.capacity() * size_of::<UpstreamAttemptRecord>()
            + record.upstream_attempts.iter().map(|attempt| attempt.upstream.capacity()).sum::<usize>();
        if let Some(diagnostics) = &record.upstream_diagnostics {
            *fields.entry("diagnostics").or_default() += diagnostics.branches.capacity() * size_of::<UpstreamDiagnosticBranch>()
                + diagnostics.branches.iter().map(|branch| branch.role.capacity() + optional(&branch.policy) + branch.decision.capacity()).sum::<usize>()
                + diagnostics.selected.as_ref().map_or(0, |selected| selected.entry.capacity())
                + diagnostics.attempts.capacity() * size_of::<UpstreamDiagnosticAttempt>()
                + diagnostics.attempts.iter().map(|attempt| attempt.entry.capacity()).sum::<usize>();
        }
        *fields.entry("failure").or_default() += match &record.failure_provenance {
            Some(FailureProvenance::UpstreamTimeout { upstream } | FailureProvenance::UpstreamFailure { upstream }) => upstream.capacity(),
            _ => 0,
        };
    }
    let containers = state.audit_records.capacity() * size_of::<Arc<AuditRecord>>()
        + state.slowest_records.capacity() * size_of::<Arc<AuditRecord>>()
        + state.audit_timings.capacity() * size_of::<AuditTimingSnapshot>();
    let payload: usize = fields.values().sum();
    eprintln!("AUDIT_MEMORY {}", serde_json::json!({
        "record_size": size_of::<AuditRecord>(),
        "response_details_size": size_of::<ResponseDetails>(),
        "diagnostics_option_size": size_of::<Option<UpstreamDiagnostics>>(),
        "failure_option_size": size_of::<Option<FailureProvenance>>(),
        "ring_len": state.audit_records.len(), "unique_records": seen.len(),
        "fields_requested_bytes": fields, "container_requested_bytes": containers,
        "total_requested_bytes": payload + containers,
        "requested_bytes_per_ring_record": (payload + containers) as f64 / state.audit_records.len().max(1) as f64,
        "excludes": "allocator size-class overhead, fragmentation, transient query/read allocations"
    }));
}
