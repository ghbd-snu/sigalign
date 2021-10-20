// Default implementations for sequence provider

use super::SequenceProvider;

mod file_reader;
mod on_memory;
mod indexed_fasta;

use file_reader::FastaReader;

pub use on_memory::OnMemoryProvider;

const A_UTF8: u8 = 65;
const C_UTF8: u8 = 67;
const G_UTF8: u8 = 71;
const T_UTF8: u8 = 84;

fn reverse_complement_of_nucleotide_sequence(sequence: &[u8]) -> Vec<u8> {
    sequence.iter().rev().map(|&character| {
        match character{
            A_UTF8 => T_UTF8,
            C_UTF8 => G_UTF8,
            G_UTF8 => C_UTF8,
            T_UTF8 => A_UTF8,
            _ => character,
        }
    }).collect()
}
