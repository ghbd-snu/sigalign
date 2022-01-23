use super::{
    Result, error_msg,
	Penalties, PRECISION_SCALE, Cutoff, MinPenaltyForPattern,
	AlignmentResult, RecordAlignmentResult, AnchorAlignmentResult, AlignmentPosition, AlignmentOperation, AlignmentCase,
    Sequence,
    ReferenceInterface, SequenceBuffer, PatternLocation,
};
use super::{
    Reference, SequenceProvider, JoinedSequence,
    SequenceType, PatternFinder,
    Serializable,
    LabelProvider,
};

use crate::util::FastaReader;

use serde::{Serialize, Deserialize};
use bincode::{serialize_into, deserialize_from};

use std::marker::PhantomData;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct InMemoryProvider<'a> {
    record_count: usize,
    combined_sequence: Vec<u8>,
    sequence_index: Vec<usize>,
    combined_label: String,
    label_index: Vec<usize>,
    _lifetime: PhantomData<&'a ()>,
}

impl<'a> InMemoryProvider<'a> {
    pub fn new() -> Self {
        Self {
            record_count: 0,
            combined_sequence: Vec::new(),
            sequence_index: vec![0],
            combined_label: String::new(),
            label_index: vec![0],
            _lifetime: PhantomData,
        }
    }
    pub fn add_record(
        &mut self,
        sequence: &[u8],
        label: &str,
    ) {
        self.record_count += 1;
        self.combined_sequence.extend_from_slice(sequence);
        self.sequence_index.push(self.combined_sequence.len());
        self.combined_label.push_str(label);
        self.label_index.push(self.combined_label.len());
    }
    pub fn add_fasta_file<P>(&mut self, file_path: P) -> Result<()> where
        P: AsRef<std::path::Path> + std::fmt::Debug,
    {
        let fasta_reader = FastaReader::from_file_path(file_path)?;
        self.add_from_fasta_reader(fasta_reader);
        Ok(())
    }
    pub fn add_fasta_bytes(&mut self, fasta_bytes: &[u8]) {
        let fasta_reader = FastaReader::from_bytes(fasta_bytes);
        self.add_from_fasta_reader(fasta_reader);
    }
    fn add_from_fasta_reader<R>(&mut self, fasta_reader: FastaReader<R>) where
        R: std::io::Read,
    {
        fasta_reader.for_each(|(label, sequence)| {
            self.add_record(&sequence, &label);
        });
    }
}

struct InMemoryBuffer<'a> {
    sequence_slice: &'a [u8],
}

impl<'a> SequenceBuffer for InMemoryBuffer<'a> {
    fn request_sequence(&self) -> &[u8] {
        self.sequence_slice
    }
}

// Sequence Provider
impl<'a> SequenceProvider<'a> for InMemoryProvider<'a> {
    type Buffer = InMemoryBuffer<'a>;

    fn total_record_count(&self) -> usize {
        self.record_count
    }
    fn get_buffer(&'a self) -> Self::Buffer {
        InMemoryBuffer {
            sequence_slice: &self.combined_sequence
        }
    }
    fn fill_sequence_buffer(&'a self, record_index: usize, buffer: &'a mut Self::Buffer) {
        buffer.sequence_slice = {
            &self.combined_sequence[
                self.sequence_index[record_index]..self.sequence_index[record_index+1]
            ]
        };
    }
    fn get_joined_sequence(&self) -> JoinedSequence {
        JoinedSequence::new(
            self.combined_sequence.to_vec(),
            self.sequence_index.iter().map(|x| *x as u64).collect(),
        )
    }
}

// // Label Provider
// impl LabelProvider for InMemoryProvider {
//     fn label_of_record(&self, record_index: usize) -> &str {
//         &self.combined_label[
//             self.label_index[record_index]..self.label_index[record_index+1]
//         ]
//     }
// }

// // Serializable
// impl Serializable for InMemoryProvider {
//     fn save_to<W>(&self, writer: W) -> Result<()> where
//         W: std::io::Write
//     {
//         serialize_into(writer, self)?;
//         Ok(())
//     }
//     fn load_from<R>(reader: R) -> Result<Self> where
//         R: std::io::Read,
//         Self: Sized,
//     {
//         let value: Self = deserialize_from(reader)?;
//         Ok(value)
//     }
// }
