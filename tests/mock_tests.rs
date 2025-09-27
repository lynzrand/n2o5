use n2o4::db::{DbReader, ExecDb};
use n2o4::{
    db::in_memory::InMemoryDb,
    exec::{BuildStatusKind, ExecConfig, Executor},
    graph::{BuildCommand, BuildMethod, BuildNode, GraphBuilder},
};

use std::path::{Path, PathBuf};

use crate::mock::{MockExecResult, MockWorld};

mod mock;

// Helper: SubCommand builder with a simple name to distinguish logs.
fn sc(name: &str) -> BuildMethod {
    BuildMethod::SubCommand(BuildCommand {
        executable: PathBuf::from(name),
        args: vec![],
    })
}

// Helper: drain world log and return a vector of labels.
fn drain_exec_labels(world: &MockWorld) -> Vec<String> {
    world
        .take_log()
        .into_iter()
        .map(|e| match e {
            MockExecResult::Subcommand(cmd) => cmd.executable.to_string_lossy().to_string(),
            MockExecResult::Callback(name) => format!("cb:{name}"),
            MockExecResult::Phony => "PHONY".to_string(),
        })
        .collect()
}

// Keep the original smoke test for an empty graph.
#[test]
fn test_nothing() {
    let cfg = ExecConfig::default();
    let graph = n2o4::graph::BuildGraph::default();
    let db = Box::new(InMemoryDb::default());
    let world = MockWorld::new();
    let mut executor = Executor::with_world(&cfg, &graph, db, &world, &());
    executor.run().unwrap();
}

// 1) Single node: Outdated -> Succeeded; assert exec log and file info written
#[test]
fn test_single_node_outdated_succeeded() {
    // Build graph: A(in.txt -> out.txt)
    let mut gb = GraphBuilder::new();
    let in_txt = gb.add_file("in.txt");
    let out_txt = gb.add_file("out.txt");
    let a = gb.add_build(BuildNode {
        command: sc("A"),
        ins: vec![in_txt],
        outs: vec![out_txt],
        restat: false,
    });
    let graph = gb.build().unwrap();

    // World and DB
    let world = MockWorld::new();
    world.touch_file(Path::new("in.txt")); // ensure input exists

    let db = InMemoryDb::default();
    let db_read = db.clone();
    let db_box: Box<dyn n2o4::db::ExecDb> = Box::new(db);

    let cfg = ExecConfig::default();
    let mut exec = Executor::with_world(&cfg, &graph, db_box, &world, &());
    exec.want([a]);
    exec.run().unwrap();

    let labels = drain_exec_labels(&world);
    assert_eq!(labels, vec!["A"]);

    let rd = db_read.begin_read();
    assert!(rd.get_file_info(Path::new("out.txt")).is_some());
}

// 2) Single node: Outdated -> Failed; assert exec log and file info invalidated
#[test]
fn test_single_node_outdated_failed() {
    // Build graph: A(in.txt -> out.txt)
    let mut gb = GraphBuilder::new();
    let in_txt = gb.add_file("in.txt");
    let out_txt = gb.add_file("out.txt");
    let a = gb.add_build(BuildNode {
        command: sc("A"),
        ins: vec![in_txt],
        outs: vec![out_txt],
        restat: false,
    });
    let graph = gb.build().unwrap();

    // World and DB
    let world = MockWorld::new();
    world.touch_file(Path::new("in.txt"));
    world.set_callback(Box::new(|_, method| {
        if let BuildMethod::SubCommand(cmd) = method
            && cmd.executable == Path::new("A")
        {
            return Ok(BuildStatusKind::Failed);
        }
        Ok(BuildStatusKind::Succeeded)
    }));

    let db = InMemoryDb::default();
    let db_read = db.clone();
    let db_box: Box<dyn n2o4::db::ExecDb> = Box::new(db);

    let cfg = ExecConfig::default();
    let mut exec = Executor::with_world(&cfg, &graph, db_box, &world, &());
    exec.want([a]);
    exec.run().unwrap();

    let labels = drain_exec_labels(&world);
    assert_eq!(labels, vec!["A"]);

    let rd = db_read.begin_read();
    // Failed build should invalidate/avoid any file info for the output
    assert!(rd.get_file_info(Path::new("out.txt")).is_none());
}

// 3) Single node: UpToDate on second run (no execution)
#[test]
fn test_single_node_up_to_date() {
    // Build graph: A(in.txt -> out.txt)
    let mut gb = GraphBuilder::new();
    let in_txt = gb.add_file("in.txt");
    let out_txt = gb.add_file("out.txt");
    let a = gb.add_build(BuildNode {
        command: sc("A"),
        ins: vec![in_txt],
        outs: vec![out_txt],
        restat: false,
    });
    let graph = gb.build().unwrap();

    let world = MockWorld::new();
    world.touch_file(Path::new("in.txt"));

    let db = InMemoryDb::default();
    let db_read = db.clone();

    // First run to populate DB
    {
        let db_box: Box<dyn n2o4::db::ExecDb> = Box::new(db.clone());
        let cfg = ExecConfig::default();
        let mut exec = Executor::with_world(&cfg, &graph, db_box, &world, &());
        exec.want([a]);
        exec.run().unwrap();
        // Drain first-run log to isolate second-run behavior
        let _ = drain_exec_labels(&world);
    }

    // Second run should be UpToDate and not execute the command
    {
        let db_box: Box<dyn n2o4::db::ExecDb> = Box::new(db_read.clone());
        let cfg = ExecConfig::default();
        let mut exec = Executor::with_world(&cfg, &graph, db_box, &world, &());
        exec.want([a]);
        exec.run().unwrap();
        let labels = drain_exec_labels(&world);
        assert!(
            labels.is_empty(),
            "Expected no execution on UpToDate, got {:?}",
            labels
        );
    }

    // File info should still exist
    let rd = db_read.begin_read();
    assert!(rd.get_file_info(Path::new("out.txt")).is_some());
}

// 4) Linear dependency: A -> B success path
#[test]
fn test_linear_dependency_success() {
    // A(a.in -> a.out), B(a.out -> b.out), with edge B depends on A
    let mut gb = GraphBuilder::new();
    let a_in = gb.add_file("a.in");
    let a_out = gb.add_file("a.out");
    let b_out = gb.add_file("b.out");

    let a = gb.add_build(BuildNode {
        command: sc("A"),
        ins: vec![a_in],
        outs: vec![a_out],
        restat: false,
    });
    let b = gb.add_build(BuildNode {
        command: sc("B"),
        ins: vec![a_out],
        outs: vec![b_out],
        restat: false,
    });
    gb.add_build_dep(b, a);
    let graph = gb.build().unwrap();

    let world = MockWorld::new();
    world.touch_file(Path::new("a.in"));
    // Ensure dependent input exists so B won't be Missing when scheduled
    world.touch_file(Path::new("a.out"));

    let db = InMemoryDb::default();
    let db_read = db.clone();
    let db_box: Box<dyn n2o4::db::ExecDb> = Box::new(db);

    let cfg = ExecConfig::default();
    let mut exec = Executor::with_world(&cfg, &graph, db_box, &world, &());
    exec.want([b]); // Want B should pull in A via dependency
    exec.run().unwrap();

    let labels = drain_exec_labels(&world);
    assert_eq!(labels.len(), 2);
    assert!(labels.contains(&"A".to_string()));
    assert!(labels.contains(&"B".to_string()));

    let rd = db_read.begin_read();
    assert!(rd.get_file_info(Path::new("b.out")).is_some());
}

// 5) Failure propagation: A Failed -> B Skipped (B not executed)
#[test]
fn test_dependency_failure_propagation_skipped() {
    // A(a.in -> a.out), B(a.out -> b.out), B depends on A
    let mut gb = GraphBuilder::new();
    let a_in = gb.add_file("a.in");
    let a_out = gb.add_file("a.out");
    let b_out = gb.add_file("b.out");

    let a = gb.add_build(BuildNode {
        command: sc("A"),
        ins: vec![a_in],
        outs: vec![a_out],
        restat: false,
    });
    let b = gb.add_build(BuildNode {
        command: sc("B"),
        ins: vec![a_out],
        outs: vec![b_out],
        restat: false,
    });
    gb.add_build_dep(b, a);
    let graph = gb.build().unwrap();

    let world = MockWorld::new();
    world.touch_file(Path::new("a.in"));
    world.set_callback(Box::new(|_, method| {
        if let BuildMethod::SubCommand(cmd) = method {
            if cmd.executable == Path::new("A") {
                return Ok(BuildStatusKind::Failed);
            }
        }
        Ok(BuildStatusKind::Succeeded)
    }));

    let db = InMemoryDb::default();
    let db_read = db.clone();
    let db_box: Box<dyn n2o4::db::ExecDb> = Box::new(db);

    let cfg = ExecConfig::default();
    let mut exec = Executor::with_world(&cfg, &graph, db_box, &world, &());
    exec.want([b]);
    exec.run().unwrap();

    let labels = drain_exec_labels(&world);
    // Only A should execute; B is skipped
    assert_eq!(labels, vec!["A"]);

    let rd = db_read.begin_read();
    // Neither A nor B should have output info (A failed -> invalidated; B skipped)
    assert!(rd.get_file_info(Path::new("a.out")).is_none());
    assert!(rd.get_file_info(Path::new("b.out")).is_none());
}

// 6) Multiple inputs gate: B executes only after A and C succeed
#[test]
fn test_multi_input_gatekeeping() {
    // A(a.in -> a.out), C(c.in -> c.out), B([a.out, c.out] -> b.out)
    let mut gb = GraphBuilder::new();
    let a_in = gb.add_file("a.in");
    let a_out = gb.add_file("a.out");
    let c_in = gb.add_file("c.in");
    let c_out = gb.add_file("c.out");
    let b_out = gb.add_file("b.out");

    let a = gb.add_build(BuildNode {
        command: sc("A"),
        ins: vec![a_in],
        outs: vec![a_out],
        restat: false,
    });
    let c = gb.add_build(BuildNode {
        command: sc("C"),
        ins: vec![c_in],
        outs: vec![c_out],
        restat: false,
    });
    let b = gb.add_build(BuildNode {
        command: sc("B"),
        ins: vec![a_out, c_out],
        outs: vec![b_out],
        restat: false,
    });
    gb.add_build_dep(b, a);
    gb.add_build_dep(b, c);
    let graph = gb.build().unwrap();

    let world = MockWorld::new();
    world.touch_file(Path::new("a.in"));
    world.touch_file(Path::new("c.in"));
    // Ensure dependent inputs exist so B won't be Missing when scheduled
    world.touch_file(Path::new("a.out"));
    world.touch_file(Path::new("c.out"));

    let db = InMemoryDb::default();
    let db_read = db.clone();
    let db_box: Box<dyn n2o4::db::ExecDb> = Box::new(db);

    let cfg = ExecConfig::default();
    let mut exec = Executor::with_world(&cfg, &graph, db_box, &world, &());
    exec.want([b]);
    exec.run().unwrap();

    let labels = drain_exec_labels(&world);
    assert_eq!(labels.len(), 3);
    assert!(labels.contains(&"A".to_string()));
    assert!(labels.contains(&"C".to_string()));
    assert!(labels.contains(&"B".to_string()));

    let rd = db_read.begin_read();
    assert!(rd.get_file_info(Path::new("b.out")).is_some());
}

// 7) Skipped chain propagation: A Failed -> B Skipped -> C Skipped (B depends on A, C depends on B)
#[test]
fn test_skipped_chain_propagation() {
    // A(a.in -> a.out) -> B(a.out -> b.out) -> C(b.out -> c.out)
    let mut gb = GraphBuilder::new();
    let a_in = gb.add_file("a.in");
    let a_out = gb.add_file("a.out");
    let b_out = gb.add_file("b.out");
    let c_out = gb.add_file("c.out");

    let a = gb.add_build(BuildNode {
        command: sc("A"),
        ins: vec![a_in],
        outs: vec![a_out],
        restat: false,
    });
    let b = gb.add_build(BuildNode {
        command: sc("B"),
        ins: vec![a_out],
        outs: vec![b_out],
        restat: false,
    });
    let c = gb.add_build(BuildNode {
        command: sc("C"),
        ins: vec![b_out],
        outs: vec![c_out],
        restat: false,
    });

    gb.add_build_dep(b, a);
    gb.add_build_dep(c, b);
    let graph = gb.build().unwrap();

    let world = MockWorld::new();
    world.touch_file(Path::new("a.in"));
    world.set_callback(Box::new(|_, method| {
        if let BuildMethod::SubCommand(cmd) = method {
            if cmd.executable == Path::new("A") {
                return Ok(BuildStatusKind::Failed);
            }
        }
        Ok(BuildStatusKind::Succeeded)
    }));

    let db = InMemoryDb::default();
    let db_read = db.clone();
    let db_box: Box<dyn n2o4::db::ExecDb> = Box::new(db);

    let cfg = ExecConfig::default();
    let mut exec = Executor::with_world(&cfg, &graph, db_box, &world, &());
    exec.want([c]); // Want C pulls in B and A
    exec.run().unwrap();

    let labels = drain_exec_labels(&world);
    // Only A executed; B and C should be skipped
    assert_eq!(labels, vec!["A"]);

    let rd = db_read.begin_read();
    assert!(rd.get_file_info(Path::new("a.out")).is_none());
    assert!(rd.get_file_info(Path::new("b.out")).is_none());
    assert!(rd.get_file_info(Path::new("c.out")).is_none());
}

// 8) Optional: parallelism=1 with two leaves; sequential execution (no strict order asserted)
#[test]
fn test_parallelism_one_two_leaves() {
    // D(d.in -> d.out), E(e.in -> e.out) 无依赖
    let mut gb = GraphBuilder::new();
    let d_in = gb.add_file("d.in");
    let d_out = gb.add_file("d.out");
    let e_in = gb.add_file("e.in");
    let e_out = gb.add_file("e.out");

    let d = gb.add_build(BuildNode {
        command: sc("D"),
        ins: vec![d_in],
        outs: vec![d_out],
        restat: false,
    });
    let e = gb.add_build(BuildNode {
        command: sc("E"),
        ins: vec![e_in],
        outs: vec![e_out],
        restat: false,
    });
    let graph = gb.build().unwrap();

    let world = MockWorld::new();
    world.touch_file(Path::new("d.in"));
    world.touch_file(Path::new("e.in"));

    let db = InMemoryDb::default();
    let db_box: Box<dyn n2o4::db::ExecDb> = Box::new(db);

    let mut cfg = ExecConfig::default();
    cfg.parallelism = 1;

    let mut exec = Executor::with_world(&cfg, &graph, db_box, &world, &());
    exec.want([d, e]);
    exec.run().unwrap();

    let labels = drain_exec_labels(&world);
    assert_eq!(labels.len(), 2);
    assert!(labels.contains(&"D".to_string()));
    assert!(labels.contains(&"E".to_string()));
}
