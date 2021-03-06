//! The `ledger` module provides functions for parallel verification of the
//! Proof of History ledger.

use bincode::{self, deserialize, serialize_into};
use entry::Entry;
use hash::Hash;
use packet::{self, SharedBlob, BLOB_SIZE};
use rayon::prelude::*;
use std::collections::VecDeque;
use std::io::Cursor;
use transaction::Transaction;

// a Block is a slice of Entries

pub trait Block {
    /// Verifies the hashes and counts of a slice of transactions are all consistent.
    fn verify(&self, start_hash: &Hash) -> bool;
    fn to_blobs(&self, blob_recycler: &packet::BlobRecycler, q: &mut VecDeque<SharedBlob>);
}

impl Block for [Entry] {
    fn verify(&self, start_hash: &Hash) -> bool {
        let genesis = [Entry::new_tick(0, start_hash)];
        let entry_pairs = genesis.par_iter().chain(self).zip(self);
        entry_pairs.all(|(x0, x1)| x1.verify(&x0.id))
    }

    fn to_blobs(&self, blob_recycler: &packet::BlobRecycler, q: &mut VecDeque<SharedBlob>) {
        for entry in self {
            let blob = blob_recycler.allocate();
            let pos = {
                let mut bd = blob.write().unwrap();
                let mut out = Cursor::new(bd.data_mut());
                serialize_into(&mut out, &entry).expect("failed to serialize output");
                out.position() as usize
            };
            assert!(pos < BLOB_SIZE);
            blob.write().unwrap().set_size(pos);
            q.push_back(blob);
        }
    }
}

pub fn reconstruct_entries_from_blobs(blobs: VecDeque<SharedBlob>) -> bincode::Result<Vec<Entry>> {
    let mut entries: Vec<Entry> = Vec::with_capacity(blobs.len());

    for blob in blobs {
        let entry = {
            let msg = blob.read().unwrap();
            deserialize(&msg.data()[..msg.meta.size])
        };

        match entry {
            Ok(entry) => entries.push(entry),
            Err(err) => {
                trace!("reconstruct_entry_from_blobs: {}", err);
                return Err(err);
            }
        }
    }
    Ok(entries)
}

/// Creates the next entries for given transactions, outputs
/// updates start_hash to id of last Entry, sets cur_hashes to 0
pub fn next_entries_mut(
    start_hash: &mut Hash,
    cur_hashes: &mut u64,
    transactions: Vec<Transaction>,
) -> Vec<Entry> {
    if transactions.is_empty() {
        vec![Entry::new_mut(start_hash, cur_hashes, transactions, false)]
    } else {
        let mut chunk_len = transactions.len();

        // check for fit, make sure they can be serialized
        while !Entry::will_fit(transactions[0..chunk_len].to_vec()) {
            chunk_len /= 2;
        }

        let mut num_chunks = if transactions.len() % chunk_len == 0 {
            transactions.len() / chunk_len
        } else {
            transactions.len() / chunk_len + 1
        };

        let mut entries = Vec::with_capacity(num_chunks);

        for chunk in transactions.chunks(chunk_len) {
            num_chunks -= 1;
            entries.push(Entry::new_mut(
                start_hash,
                cur_hashes,
                chunk.to_vec(),
                num_chunks > 0,
            ));
        }
        entries
    }
}

/// Creates the next Entries for given transactions
pub fn next_entries(
    start_hash: &Hash,
    cur_hashes: u64,
    transactions: Vec<Transaction>,
) -> Vec<Entry> {
    let mut id = *start_hash;
    let mut num_hashes = cur_hashes;
    next_entries_mut(&mut id, &mut num_hashes, transactions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use entry::{next_entry, Entry};
    use hash::hash;
    use packet::{BlobRecycler, BLOB_DATA_SIZE};
    use signature::{KeyPair, KeyPairUtil};
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use transaction::Transaction;

    #[test]
    fn test_verify_slice() {
        let zero = Hash::default();
        let one = hash(&zero);
        assert!(vec![][..].verify(&zero)); // base case
        assert!(vec![Entry::new_tick(0, &zero)][..].verify(&zero)); // singleton case 1
        assert!(!vec![Entry::new_tick(0, &zero)][..].verify(&one)); // singleton case 2, bad
        assert!(vec![next_entry(&zero, 0, vec![]); 2][..].verify(&zero)); // inductive step

        let mut bad_ticks = vec![next_entry(&zero, 0, vec![]); 2];
        bad_ticks[1].id = one;
        assert!(!bad_ticks.verify(&zero)); // inductive step, bad
    }

    #[test]
    fn test_entries_to_blobs() {
        let zero = Hash::default();
        let one = hash(&zero);
        let keypair = KeyPair::new();
        let tx0 = Transaction::new(&keypair, keypair.pubkey(), 1, one);
        let transactions = vec![tx0; 10_000];
        let entries = next_entries(&zero, 0, transactions);

        let blob_recycler = BlobRecycler::default();
        let mut blob_q = VecDeque::new();
        entries.to_blobs(&blob_recycler, &mut blob_q);

        assert_eq!(reconstruct_entries_from_blobs(blob_q).unwrap(), entries);
    }

    #[test]
    fn test_bad_blobs_attack() {
        let blob_recycler = BlobRecycler::default();
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), 8000);
        let blobs_q = packet::to_blobs(vec![(0, addr)], &blob_recycler).unwrap(); // <-- attack!
        assert!(reconstruct_entries_from_blobs(blobs_q).is_err());
    }

    #[test]
    fn test_next_entries() {
        let id = Hash::default();
        let next_id = hash(&id);
        let keypair = KeyPair::new();
        let tx0 = Transaction::new(&keypair, keypair.pubkey(), 1, next_id);

        // NOTE: if Entry grows to larger than a transaction, the code below falls over
        let threshold = (BLOB_DATA_SIZE / 256) - 1; // 256 is transaction size

        // verify no split
        let transactions = vec![tx0.clone(); threshold];
        let entries0 = next_entries(&id, 0, transactions.clone());
        assert_eq!(entries0.len(), 1);
        assert!(entries0.verify(&id));

        // verify the split
        let transactions = vec![tx0.clone(); threshold * 2];
        let entries0 = next_entries(&id, 0, transactions.clone());
        assert_eq!(entries0.len(), 2);
        assert!(entries0[0].has_more);
        assert!(!entries0[entries0.len() - 1].has_more);

        assert!(entries0.verify(&id));
        // test hand-construction... brittle, changes if split method changes... ?
        //        let mut entries1 = vec![];
        //        entries1.push(Entry::new(&id, 1, transactions[..threshold].to_vec(), true));
        //        id = entries1[0].id;
        //        entries1.push(Entry::new(
        //            &id,
        //            1,
        //            transactions[threshold..].to_vec(),
        //            false,
        //        ));
        //
        //        assert_eq!(entries0, entries1);
    }
}

#[cfg(all(feature = "unstable", test))]
mod bench {
    extern crate test;
    use self::test::Bencher;
    use hash::hash;
    use ledger::*;
    use packet::BlobRecycler;
    use signature::{KeyPair, KeyPairUtil};
    use transaction::Transaction;

    #[bench]
    fn bench_block_to_blobs_to_block(bencher: &mut Bencher) {
        let zero = Hash::default();
        let one = hash(&zero);
        let keypair = KeyPair::new();
        let tx0 = Transaction::new(&keypair, keypair.pubkey(), 1, one);
        let transactions = vec![tx0; 10];
        let entries = next_entries(&zero, 1, transactions);

        let blob_recycler = BlobRecycler::default();
        bencher.iter(|| {
            let mut blob_q = VecDeque::new();
            entries.to_blobs(&blob_recycler, &mut blob_q);
            assert_eq!(reconstruct_entries_from_blobs(blob_q).unwrap(), entries);
        });
    }

}
