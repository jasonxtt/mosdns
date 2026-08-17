//! Pure Rust domain matcher core for MosDNS.
//!
//! Implements the four Go domain matcher types (full, domain/suffix, keyword,
//! regex) and their `MixMatcher` combination with precedence
//! full → domain → regex → keyword. Normalization matches Go's
//! `NormalizeDomain` (lowercase + strip trailing dot).
//!
//! This crate is an `rlib` linked by `mosdns-runtime` (the sole `staticlib`).

#![allow(clippy::pedantic)]

mod full;
mod ipnet;
mod keyword;
mod mix;
mod normalize;
mod regex;
mod trie;
mod valued;

pub use full::FullMatcher;
pub use ipnet::IpPrefixList;
pub use keyword::KeywordMatcher;
pub use mix::MixMatcher;
pub use normalize::normalize;
pub use regex::{RegexBuildError, RegexMatcher};
pub use trie::DomainSuffixMatcher;
pub use valued::{
    ValuedBuildError, ValuedDomainMatcher, ValuedEncodingError, ValuedMatchResult, ValuedRule,
    decode_valued_match_result, decode_valued_rule_batch, encode_valued_match_result,
    encode_valued_rule_batch,
};

/// Domain matcher trait, generic over the result value.
///
/// Return of `None` means no match. Boolean matchers use `()` as the value.
pub trait DomainMatcher {
    type Value;
    fn r#match(&self, domain: &str) -> Option<&Self::Value>;
}
