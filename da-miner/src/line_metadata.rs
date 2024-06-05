use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

use ethers::types::U256;
use std::time::Instant;
use storage::slice_db::{BlobInfo, SliceDB, SliceIndex};

use crate::{line_candidate::LineCandidate, mine::calculate_line_quality, watcher::SampleTask};

type EpochInfo = BTreeSet<BlobInfo>;

#[derive(Default)]
pub(crate) struct LineMetadata {
    data: BTreeMap<u64, EpochInfo>,
    epoch_to_fetch: BTreeSet<u64>,
}

impl LineMetadata {
    pub fn needs_fetch(&self) -> bool {
        !self.epoch_to_fetch.is_empty()
    }

    pub fn new_epoch(&mut self, epoch: u64) {
        self.epoch_to_fetch.insert(epoch);
    }

    pub fn new_epoch_range(&mut self, epoches: impl IntoIterator<Item = u64>) {
        for e in epoches {
            self.epoch_to_fetch.insert(e);
        }
    }

    pub async fn fetch_epoch(
        &mut self,
        db: &impl SliceDB,
        duration: Duration,
    ) -> Result<(), String> {
        let deadline = Instant::now() + duration;

        while Instant::now() < deadline {
            let next_epoch = if let Some(x) = self.epoch_to_fetch.pop_first() {
                x
            } else {
                break;
            };

            if self.data.contains_key(&next_epoch) {
                continue;
            }

            let epoch_info = db
                .get_epoch_info(next_epoch)
                .await
                .map_err(|e| format!("Fail to fetch epoch {}: {:?}", next_epoch, e))?;

            if !epoch_info.is_empty() {
                self.data.insert(next_epoch, epoch_info);
            }
        }
        Ok(())
    }

    pub fn iter_next_epoch(
        &self,
        start_epoch: u64,
        num_batch: usize,
        task: SampleTask,
    ) -> (Vec<LineCandidate>, Option<u64>) {
        if self
            .data
            .last_key_value()
            .map_or(true, |(&epoch, _)| epoch < start_epoch)
        {
            return (vec![], None);
        }

        let mut answer = vec![];
        let mut last_epoch = 0;

        let mut max_quality = [0u8; 32];
        task.quality.to_big_endian(&mut max_quality);

        for (&epoch, blobs) in self.data.range(start_epoch..).take(num_batch) {
            for blob in blobs.iter() {
                let quorum_id = blob.quorum_id;
                let storage_root = blob.storage_root;
                for &index in &blob.indicies {
                    let line_quality =
                        calculate_line_quality(task.hash, epoch, quorum_id, storage_root, index);
                    if line_quality <= max_quality {
                        answer.push(LineCandidate::new(
                            SliceIndex {
                                epoch,
                                quorum_id,
                                storage_root,
                                index: index as u64,
                            },
                            task,
                            U256::from_big_endian(&line_quality),
                        ));
                    }
                }
            }
            last_epoch = epoch;
        }

        (answer, Some(last_epoch))
    }
}
