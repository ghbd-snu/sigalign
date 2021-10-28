/*!
# sigalign
## Similarity Guided Alignment Algorithm
---
# Quick Start
```rust
use sigalign::{Reference, Aligner};
use sigalign::reference::InMemoryProvider;

// (1) Make `Reference`
let mut sequence_provider = InMemoryProvider::new_empty();
sequence_provider.add_labeled_sequence(
    "record_1".to_string(),
    b"ATCAAACTCACAATTGTATTTCTTTGCCAGCTGGGCATATACTTTTTCCGCACCCTCATTTAACTTCTTGGATAACGGAAGCACACCGATCTTAACCGGAGCAAGTGCCGGATGAAAATGGAAAACGGTTCTTACGTCCGGCTTTTCCTCTGTTCCGATATTTTCCTCATCGTATGCAGCACATAAAAATGCCAGAACCA".to_vec(),
);
sequence_provider.add_labeled_sequence(
    "record_2".to_string(),
    b"TTCCATCAAACTCACAATTGTATTTCTTTGCCAGCTGGGCATATACTTTTTCCGCACCCTCATTTAACTTCTTGGATAACGGAAGCACACCGATCTTAACCGGAGCGTATGCAGCACATAAAAAT".to_vec(),
);
let mut reference = Reference::new_with_default_config(sequence_provider).unwrap();

// (2) Make `Aligner`
let aligner = Aligner::new(4, 6, 2, 100, 0.1).unwrap();

// (3) Alignment with query
let query = b"TTCCTCTGTCATCAAACTCACAATTGTATTTCTTTGCCAGCTGGGCATATACTTTTTCCGCCCCCTCATTTAACTTCTTGGATAACGGAAGCACACCGATCTTAACCGGAGGTGCCGGATGAAAATGGAAAACGGTTCTTACGTCCGGCTTTTCCTCTGTTCCGATATTTTCCTCAT";
// - Semi-global alignment
let result_semi_global: String = aligner.semi_global_alignment_labeled(&mut reference, query).unwrap();
// - Local alignment
let result_local: String = aligner.local_alignment_labeled(&mut reference, query).unwrap();
```
*/
use anyhow::{Result, bail as error_msg};
use serde::{Serialize, Deserialize, de::DeserializeOwned};

#[doc(hidden)]
// Core
mod core;
#[doc(hidden)]
// Algorithm
mod algorithm;
/// Configurations for [Reference]
pub mod reference;
/// Aligner
pub mod aligner;
#[doc(hidden)]
pub mod deprecated;
#[cfg(test)]
mod tests;

/// Reference
pub use reference::Reference;
/// Aligner
pub use aligner::Aligner;