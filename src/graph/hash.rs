//! Hashing identity of builds and their input sets.

use twox_hash::XxHash3_128;

use crate::{
    db::BuildHash,
    graph::{BuildGraph, BuildId, BuildMethod, BuildNode},
};

/// Generate a identity hash for this build.
///
/// The hash is independent of the actual layout of the graph or the build,
/// e.g. the [`FileId`]s used to represent files. However, it is still
/// sensitive to the order of output files.
pub fn hash_build(node: &BuildNode, graph: &BuildGraph) -> BuildHash {
    let mut hasher = XxHash3_128::new();

    match &node.command {
        BuildMethod::SubCommand(build_command) => {
            hasher.write(b"subcmd\0");
            hasher.write(build_command.executable.as_os_str().as_encoded_bytes());
            hasher.write(&[0]);
            for arg in &build_command.args {
                hasher.write(arg.as_encoded_bytes());
                hasher.write(&[0]);
            }
        }
        BuildMethod::Callback(s, _) => {
            // Note: only the name is hashed, not the function pointer.
            hasher.write(b"callback\0");
            hasher.write(s.as_bytes());
        }
        BuildMethod::Phony => hasher.write(b"phony\0"),
    }

    hasher.write(b"out\0");
    for &file_id in &node.outs {
        let path = graph.lookup_path(file_id).expect("invalid FileId");
        hasher.write(path.as_os_str().as_encoded_bytes());
        hasher.write(&[0]);
    }

    let res = hasher.finish_128();
    BuildHash(res.to_be_bytes())
}

/// Hash the input set of a build node.
///
/// This hash is order-independent, to mitigate the difference layout of the
/// graph between runs.
pub fn hash_input_set(build_id: BuildId, graph: &BuildGraph) -> [u8; 32] {
    let mut acc = Acc::default();
    let build = graph.lookup_build(build_id).expect("invalid BuildId");

    // Fixed inputs
    for &file_id in &build.ins {
        let path = graph.lookup_path(file_id).expect("invalid FileId");
        let h = blake3::hash(path.as_os_str().as_encoded_bytes());
        acc.accumulate(h);
    }

    // Dependency inputs
    for dep in graph.build_dependencies(build_id) {
        let dep = graph.lookup_build(dep).expect("invalid BuildId");
        for &out in &dep.outs {
            let path = graph.lookup_path(out).expect("invalid FileId");
            let h = blake3::hash(path.as_os_str().as_encoded_bytes());
            acc.accumulate(h);
        }
    }

    acc.finalize()
}

/// The accumulator for collecting an order-independent hash of input files
#[derive(Default)]
struct Acc {
    sum_lo: u128,
    sum_hi: u128,
    xor_lo: u128,
    xor_hi: u128,
    cnt: usize,
}

impl Acc {
    fn accumulate(&mut self, h: blake3::Hash) {
        let (lo, hi) = h.as_bytes().split_at(16);
        let lo = u128::from_le_bytes(lo.try_into().expect("hash length"));
        let hi = u128::from_le_bytes(hi.try_into().expect("hash length"));

        self.sum_lo = self.sum_lo.wrapping_add(lo);
        self.sum_hi = self.sum_hi.wrapping_add(hi);
        self.xor_lo ^= lo;
        self.xor_hi ^= hi;
        self.cnt += 1;
    }

    fn finalize(&self) -> [u8; 32] {
        let mut b3 = blake3::Hasher::new();
        b3.update(&self.sum_lo.to_le_bytes());
        b3.update(&self.sum_hi.to_le_bytes());
        b3.update(&self.xor_lo.to_le_bytes());
        b3.update(&self.xor_hi.to_le_bytes());
        b3.update(&self.cnt.to_le_bytes());
        let h = b3.finalize();
        *h.as_bytes()
    }
}
