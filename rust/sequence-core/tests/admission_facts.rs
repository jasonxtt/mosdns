use std::collections::BTreeMap;
use std::sync::Arc;

use mosdns_dns_core::{QueryHeader, QuestionInfo};
use mosdns_sequence_core::{AdmissionFacts, ExecutionState, StateSnapshot};

fn state() -> ExecutionState {
    ExecutionState::new(
        QueryHeader {
            id: 1,
            qr: false,
            opcode: 0,
            qdcount: 1,
            ancount: 0,
            nscount: 0,
            arcount: 0,
        },
        QuestionInfo {
            qname_wire: vec![0],
            qtype: 1,
            qclass: 1,
        },
    )
}

fn facts(entries: &[(u32, &str)]) -> AdmissionFacts {
    Arc::new(
        entries
            .iter()
            .map(|(key, value)| (*key, Arc::from(*value)))
            .collect::<BTreeMap<_, _>>(),
    )
}

#[test]
fn a_fresh_state_has_no_admission_values() {
    let state = state();
    assert!(state.admission_value(0).is_none());
    assert!(state.admission_value(16).is_none());
    assert_eq!(state.admission_facts, AdmissionFacts::default());
}

#[test]
fn admission_values_are_shared_by_snapshot_and_clone_without_copying_entries() {
    let mut state = state();
    let admission = facts(&[(0, "A"), (14, "custom value"), (16, "")]);
    let original = admission.clone();
    state.set_admission_facts(admission);

    let cloned = state.clone();
    let snapshot: StateSnapshot = state.snapshot();

    // Cloning and snapshotting share the same immutable map; the values are
    // observed through the shared allocation rather than rebuilt copies.
    assert!(Arc::ptr_eq(&cloned.admission_facts, &original));
    assert!(Arc::ptr_eq(&snapshot.admission_facts, &original));

    assert_eq!(state.admission_value(0), Some("A"));
    assert_eq!(state.admission_value(14), Some("custom value"));
    assert_eq!(state.admission_value(16), Some(""));
    assert_eq!(cloned.admission_value(14), Some("custom value"));
    assert_eq!(snapshot.admission_value(14), Some("custom value"));
    assert!(state.admission_value(9).is_none());
}

#[test]
fn mutation_channels_never_touch_the_admission_facts() {
    let mut state = state();
    state.set_admission_facts(facts(&[(3, "B")]));
    let before = state.admission_facts.clone();

    state.fast_flags = u64::MAX;
    state.marks.insert(7);
    state.set_raw_response(vec![0, 1, 2]);
    state.apply_mutation(mosdns_sequence_core::StateMutation::SetFastFlags(0));

    assert!(Arc::ptr_eq(&state.admission_facts, &before));
    assert_eq!(state.admission_value(3), Some("B"));
}

#[test]
fn snapshot_equality_includes_the_admission_facts() {
    let mut first = state();
    first.set_admission_facts(facts(&[(1, "A")]));

    let second = state();
    assert_ne!(first.snapshot(), second.snapshot());

    let mut third = state();
    third.set_admission_facts(facts(&[(1, "A")]));
    assert_eq!(first.snapshot(), third.snapshot());
}
