use super::{
    Result, error_msg,
	Penalties, PRECISION_SCALE, Cutoff, MinPenaltyForPattern,
	ReferenceAlignmentResult, RecordAlignmentResult, AlignmentResult, AlignmentPosition, AlignmentOperation, AlignmentType,
    Sequence,
    ReferenceInterface, PatternLocation,
    AlignerInterface,
};

use super::{
    SequenceType,
    SizeAwareEncoding,
};

mod fm_index;
use lt_fm_index::{LtFmIndex, LtFmIndexBuilder};

use std::collections::HashMap;
use std::fmt::Debug;
use std::io::{Write, Read};

#[derive(Clone, PartialEq, Eq)]
pub struct PatternFinder {
    lt_fm_index: LtFmIndex,
    record_boundary_positions: Vec<u64>,
}

impl PatternFinder {
    pub fn new(
        joined_sequence: JoinedSequence,
        is_nucleotide: bool,
        with_noise: bool,
        compress_rank_check_point_with_128: bool,
        suffix_array_sampling_ratio: u64,
        kmer_size_for_lookup_table: usize,
    ) -> Result<Self> {
        let mut lt_fm_index_builder = LtFmIndexBuilder::new()
            .set_lookup_table_kmer_size(kmer_size_for_lookup_table)?
            .set_suffix_array_sampling_ratio(suffix_array_sampling_ratio)?;
        
        lt_fm_index_builder = if is_nucleotide {
            if with_noise {
                lt_fm_index_builder.use_nucleotide_with_noise()
            } else {
                lt_fm_index_builder.use_nucleotide_only()
            }
        } else {
            if with_noise {
                lt_fm_index_builder.use_amino_acid_with_noise()
            } else {
                lt_fm_index_builder.use_amino_acid_only()
            }
        };

        lt_fm_index_builder = if compress_rank_check_point_with_128 {
            lt_fm_index_builder.compress_bwt_128()
        } else {
            lt_fm_index_builder.compress_bwt_64()
        };

        let lt_fm_index = lt_fm_index_builder.build(joined_sequence.bytes);

        Ok(Self {
            lt_fm_index,
            record_boundary_positions: joined_sequence.record_boundary_positions,
        })
    }
    pub fn locate_in_record_search_range(&self, pattern: Sequence, target_record_index: &[u32]) -> Vec<PatternLocation> {
        let sorted_locations = self.sorted_locations_of_pattern(pattern);

        let mut positions_by_record: HashMap<usize, Vec<usize>> = HashMap::new();
        // TODO: (1) Apply capacity (2) Change to faster hasher

        let pattern_size = pattern.len() as u64;
        let search_range_count = target_record_index.len();

        let mut size;
        let mut left;
        let mut right;
        let mut mid = 0;
        let mut index;

        for position in sorted_locations {
            // reset
            right = search_range_count;
            left = mid;
            size = right - left;
    
            while left < right {
                mid = left + size / 2;
                index = target_record_index[mid] as usize;
                
                let start = self.record_boundary_positions[index];
                let end = self.record_boundary_positions[index + 1];

                if position >= end {
                    left = mid + 1;
                } else if start > position {
                    right = mid;
                } else {
                    if (position + pattern_size) < end {
                        let ref_pos = (position - start) as usize;
                        match positions_by_record.get_mut(&index) {
                            Some(v) => {
                                v.push(ref_pos);
                            },
                            None => {
                                positions_by_record.insert(index, vec![ref_pos]);
                            },
                        }
                        break;
                    } else {
                        break;
                    }
                }
    
                size = right - left;
            }
        }
    
        positions_by_record.into_iter().map(|(record_index, positions)| {
            PatternLocation {
                record_index: record_index,
                positions: positions,
            }
        }).collect()
    }
    fn sorted_locations_of_pattern(&self, pattern: Sequence) -> Vec<u64> {
        let mut locations = self.lt_fm_index.locate(pattern);
        locations.sort();
        locations
    }
}

impl Debug for PatternFinder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PatternFinder")
            .field("lt_fm_index", &self.lt_fm_index)
            .field("record_boundary_positions_length", &self.record_boundary_positions.len())
            .finish()
    }
}

use crate::{EndianType};
use byteorder::{ReadBytesExt, WriteBytesExt};

impl SizeAwareEncoding for PatternFinder {
    fn save_to<W>(&self, mut writer: W) -> Result<()> where
        W: Write,
    {
        // Write size information
        let lt_fm_index_inner_bytes_size = self.lt_fm_index.inner_bytes_size() as u64;
        let record_boundary_positions_size = self.record_boundary_positions.len() as u64;

        writer.write_u64::<EndianType>(lt_fm_index_inner_bytes_size)?;
        writer.write_u64::<EndianType>(record_boundary_positions_size)?;
        
        // Write lt-fm-index
        self.lt_fm_index.save_to(&mut writer)?;
        // Write record_boundary_positions
        self.record_boundary_positions.iter().for_each(|position| {
            writer.write_u64::<EndianType>(*position);
        });
        Ok(())
    }
    fn load_from<R>(mut reader: R) -> Result<Self> where
        R: Read,
        Self: Sized,
    {
        // Read size information
        let lt_fm_index_size = reader.read_u64::<EndianType>()? as usize;
        let record_boundary_positions_size = reader.read_u64::<EndianType>()? as usize;
        
        // Read lt-fm-index
        let mut lt_fm_index_vector: Vec<u8> = vec![0; lt_fm_index_size];
        reader.read_exact(&mut lt_fm_index_vector)?;
        let lt_fm_index = LtFmIndex::new_from_bytes_checked(lt_fm_index_vector)?;

        // Read record boundary position
        let mut record_boundary_positions: Vec<u64> = vec![0; record_boundary_positions_size];
        reader.read_u64_into::<EndianType>(&mut record_boundary_positions)?;

        Ok(Self {
            lt_fm_index,
            record_boundary_positions,
        })
    }
}

// Contain two vectors necessary to create `PatternIndex`.
//   - The "joined_sequence" means the sequence of concatenated sequences of all record.
//   - The "accumulated_lengths" means the accumulated sequence lengths from 0 to the sum of the lengths of all sequences.
//   - For examples, if there are three records with "ATT", "CC", "GGGG", the "joined_sequence" is "ATTCCGGGG" and the "accumulated_lengths" is [0, 3, 5, 9].
pub struct JoinedSequence {
    bytes: Vec<u8>,
    record_boundary_positions: Vec<u64>,
}

impl JoinedSequence {
    pub fn new(bytes: Vec<u8>, record_boundary_positions: Vec<u64>) -> Self {
        Self {
            bytes,
            record_boundary_positions,
        }
    }
}
