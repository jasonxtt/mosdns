// Diagnostic-only include inside observer/retention.rs in the disposable
// candidate source. Call retention::audit_memory_report(&state) before cloning.
pub(super) fn audit_memory_report(state: &super::ObserverState) {
    use std::collections::{BTreeMap, BTreeSet};
    use std::mem::size_of;

    fn optional(value: &Option<String>) -> usize {
        value.as_ref().map_or(0, String::capacity)
    }
    let mut fields = BTreeMap::<&str, usize>::new();
    let (mut rows, mut templates, mut responses) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
    for row in state.audit_records.iter().chain(&state.slowest_records) {
        if !rows.insert(Arc::as_ptr(row)) { continue; }
        *fields.entry("row_inline_and_arc").or_default() += size_of::<RetainedAuditRecord>() + 16;
        *fields.entry("qname").or_default() += row.qname.len();
        if let RetainedTraceId::Text(value) = &row.trace_id {
            *fields.entry("trace_fallback").or_default() += value.len();
        }
        if templates.insert(Arc::as_ptr(&row.template)) {
            let template = &row.template;
            *fields.entry("template_inline_and_arc").or_default() += size_of::<AuditRecord>() + 16;
            for (name, value) in [
                ("final_sequence", &template.final_sequence), ("matched_group", &template.matched_group),
                ("domain_set", &template.domain_set), ("effective_tag", &template.effective_tag),
                ("matched_rule_source", &template.matched_rule_source), ("final_upstream", &template.final_upstream),
                ("upstream_targets", &template.upstream_targets), ("selected_upstream", &template.selected_upstream),
            ] {
                *fields.entry(name).or_default() += optional(value);
            }
            *fields.entry("response_source").or_default() += match &template.response {
                super::ResponseState::Dns { source: super::ResponseSource::Upstream(value), .. } => value.capacity(),
                _ => 0,
            };
            *fields.entry("upstream_attempts").or_default() += template.upstream_attempts.capacity() * size_of::<super::UpstreamAttemptRecord>()
                + template.upstream_attempts.iter().map(|attempt| attempt.upstream.capacity()).sum::<usize>();
            if let Some(diagnostics) = &template.upstream_diagnostics {
                *fields.entry("diagnostics").or_default() += diagnostics.branches.capacity() * size_of::<super::UpstreamDiagnosticBranch>()
                    + diagnostics.branches.iter().map(|branch| branch.role.capacity() + optional(&branch.policy) + branch.decision.capacity()).sum::<usize>()
                    + diagnostics.selected.as_ref().map_or(0, |selected| selected.entry.capacity())
                    + diagnostics.attempts.capacity() * size_of::<super::UpstreamDiagnosticAttempt>()
                    + diagnostics.attempts.iter().map(|attempt| attempt.entry.capacity()).sum::<usize>();
            }
            *fields.entry("failure").or_default() += match &template.failure_provenance {
                Some(super::FailureProvenance::UpstreamTimeout { upstream } | super::FailureProvenance::UpstreamFailure { upstream }) => upstream.capacity(),
                _ => 0,
            };
        }
        if responses.insert(Arc::as_ptr(&row.response_details)) {
            let response = &row.response_details;
            *fields.entry("response_inline_and_arc").or_default() += size_of::<ResponseDetails>() + 16;
            *fields.entry("answer_vec").or_default() += response.answers.capacity() * size_of::<super::AuditAnswer>();
            *fields.entry("answer_data").or_default() += response.answers.iter().map(|answer| answer.data.capacity()).sum::<usize>();
            *fields.entry("answer_decode_error").or_default() += optional(&response.answer_decode_error);
        }
    }
    let cache = &state.retention;
    *fields.entry("stale_weak_allocations").or_default() += cache.templates.slots.iter().flatten().filter(|weak| weak.strong_count() == 0).count() * (size_of::<AuditRecord>() + 16)
        + cache.responses.slots.iter().flatten().filter(|weak| weak.strong_count() == 0).count() * (size_of::<ResponseDetails>() + 16);
    let containers = state.audit_records.capacity() * size_of::<Arc<RetainedAuditRecord>>()
        + state.slowest_records.capacity() * size_of::<Arc<RetainedAuditRecord>>()
        + state.audit_timings.capacity() * size_of::<super::AuditTimingSnapshot>()
        + cache.templates.slots.capacity() * size_of::<Option<Weak<AuditRecord>>>()
        + cache.responses.slots.capacity() * size_of::<Option<Weak<ResponseDetails>>>();
    let payload: usize = fields.values().sum();
    eprintln!("AUDIT_MEMORY {}", serde_json::json!({
        "record_size": size_of::<RetainedAuditRecord>(), "ring_len": state.audit_records.len(),
        "unique_records": rows.len(), "unique_templates": templates.len(), "unique_responses": responses.len(),
        "fields_requested_bytes": fields, "container_requested_bytes": containers,
        "total_requested_bytes": payload + containers,
        "requested_bytes_per_ring_record": (payload + containers) as f64 / state.audit_records.len().max(1) as f64,
        "excludes": "allocator size-class overhead, fragmentation, transient query/read allocations"
    }));
}
