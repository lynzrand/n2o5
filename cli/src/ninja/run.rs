use std::{collections::HashSet, path::Path};

use n2o5::graph::BuildId;

use crate::ninja::model::NinjaFile;

use super::convert::ConvertOutput;

pub fn resolve_targets_to_build_ids<'s>(
    targets: &[String],
    parsed: &NinjaFile<'s>,
    converted: &ConvertOutput,
) -> Vec<BuildId> {
    let mut wanted: HashSet<BuildId> = HashSet::new();
    let mut phony_visited: HashSet<String> = HashSet::new();

    if targets.is_empty() {
        if !parsed.defaults.is_empty() {
            for t in &parsed.defaults {
                add_target_rec(
                    t.as_ref(),
                    parsed,
                    converted,
                    &mut wanted,
                    &mut phony_visited,
                );
            }
        } else {
            // No explicit targets and no defaults: build everything
            for (id, _) in converted.graph.nodes() {
                wanted.insert(id);
            }
        }
    } else {
        for t in targets {
            add_target_rec(
                t.as_str(),
                parsed,
                converted,
                &mut wanted,
                &mut phony_visited,
            );
        }
    }

    wanted.into_iter().collect()
}

fn add_target_rec<'s>(
    name: &str,
    parsed: &NinjaFile<'s>,
    converted: &ConvertOutput,
    wanted: &mut HashSet<BuildId>,
    phony_visited: &mut HashSet<String>,
) {
    // If a real file target exists and is produced by a build, add that build
    if let Some(fid) = converted.graph.lookup_fileid(Path::new(name))
        && let Some(&bid) = converted.file_to_build.get(&fid)
    {
        wanted.insert(bid);
        return;
    }
    // Otherwise, if it's a phony target, expand recursively into its inputs
    if let Some(phony) = parsed.phony.get(name) {
        if !phony_visited.insert(name.to_string()) {
            return; // already visited
        }
        // Build phony means building its inputs; expand recursively
        for i in &phony.inputs {
            add_target_rec(i.as_ref(), parsed, converted, wanted, phony_visited);
        }
        for i in &phony.implicit_inputs {
            add_target_rec(i.as_ref(), parsed, converted, wanted, phony_visited);
        }
        for i in &phony.order_only_inputs {
            add_target_rec(i.as_ref(), parsed, converted, wanted, phony_visited);
        }
    }
}
