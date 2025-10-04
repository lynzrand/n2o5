use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
};

use n2o4::graph::{BuildCommand, BuildId, BuildMethod, BuildNode, FileId, GraphBuilder};

use crate::ninja::model::{Build, NinjaFile};

/// Convert a ninja build file to n2o4 in-memory graph
pub fn ninja_to_n2o4(ninja: &NinjaFile<'_>) -> anyhow::Result<ConvertOutput> {
    let mut cx = ConvertCtx {
        ninja,
        builder: GraphBuilder::new(),
        build_out_files: HashMap::new(),
        pending_inputs: HashMap::new(),
    };

    for build in &ninja.builds {
        translate_build(&mut cx, build);
    }

    let graph = cx.builder.build()?;
    Ok(ConvertOutput {
        graph,
        file_to_build: cx.build_out_files,
    })
}

pub struct ConvertOutput {
    pub graph: n2o4::graph::BuildGraph,
    pub file_to_build: HashMap<FileId, BuildId>,
}

struct ConvertCtx<'a, 's> {
    ninja: &'a NinjaFile<'s>,
    builder: GraphBuilder,
    /// The output files associated with each build
    build_out_files: HashMap<FileId, BuildId>,
    /// Inputs that are not yet declared as output of any build
    pending_inputs: HashMap<FileId, Vec<BuildId>>,
}

/// Translates a ninja build to a build node.
fn translate_build(ctx: &mut ConvertCtx, build: &Build) {
    // Panic when any build has features we don't know
    assert!(build.rspfile.is_none());
    assert!(build.rspfile_content.is_none());
    // assert!(!build.restat);

    // Resolve input files
    let mut ins = vec![];
    let mut order_only_ins = vec![];
    for input in build.inputs.iter().chain(&build.implicit_inputs) {
        rec_desugar_possible_phony(ctx, &mut ins, Some(&mut order_only_ins), input);
    }
    for input in &build.order_only_inputs {
        rec_desugar_possible_phony(ctx, &mut order_only_ins, None, input);
    }

    // Resolve output files
    let mut outs = vec![];
    for out in build.outputs.iter().chain(&build.implicit_outputs) {
        rec_desugar_possible_phony(ctx, &mut outs, None, out);
    }

    // Create command
    let cmd = BuildCommand {
        executable: "sh".into(),
        args: vec![
            OsStr::new("-c").into(),
            OsString::from(build.command.clone().into_owned()).into(),
        ],
    };
    let node = BuildNode {
        command: BuildMethod::SubCommand(cmd),
        ins: ins.clone(),
        outs: outs.clone(),
    };
    let id = ctx.builder.add_build(node);

    // Announce outputs
    for out in outs {
        ctx.build_out_files.insert(out, id);
        // Check if anyone was waiting for this input
        if let Some(pending) = ctx.pending_inputs.remove(&out) {
            for dep_id in pending {
                ctx.builder.add_build_dep(dep_id, id);
            }
        }
    }
    // Announce inputs or deps
    for input in ins.into_iter().chain(order_only_ins) {
        if let Some(&prod) = ctx.build_out_files.get(&input) {
            // This input is produced by a known build
            ctx.builder.add_build_dep(id, prod);
        } else {
            // This input is not yet produced by any known build
            ctx.pending_inputs.entry(input).or_default().push(id);
        }
    }
}

fn rec_desugar_possible_phony(
    ctx: &mut ConvertCtx,
    out: &mut Vec<FileId>,
    mut order_only: Option<&mut Vec<FileId>>,
    file: &str,
) {
    if let Some(phony) = ctx.ninja.phony.get(file) {
        for input in &phony.inputs {
            rec_desugar_possible_phony(ctx, out, order_only.as_deref_mut(), input);
        }
        for input in &phony.implicit_inputs {
            rec_desugar_possible_phony(ctx, out, order_only.as_deref_mut(), input);
        }
        for input in &phony.order_only_inputs {
            let order_only_out = order_only.as_deref_mut().unwrap_or(out);
            rec_desugar_possible_phony(ctx, order_only_out, None, input);
        }
    } else {
        let fid = ctx.builder.add_file(file);
        out.push(fid);
    }
}
