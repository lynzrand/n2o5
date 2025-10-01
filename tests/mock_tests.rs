//! A number of minimal mocking tests for the basic functionality of the executor.
//!
//! Most of these tests are written by an LLM. They are very small tests, so
//! it's acceptable.

use n2o4::db::ExecDb;
use n2o4::{
    db::in_memory::InMemoryDb,
    exec::{BuildStatusKind, ExecConfig, Executor},
    graph::BuildMethod,
};

use test_log::test;

use std::path::Path;

use crate::mock::{MockExecResult, MockWorld};

mod mock;

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

macro_rules! mock_graph {
    (
        $(
            // Rule name
            $id:ident
            // Rule , dependencies
            $(, dep($($dep:ident),*$(,)?))? :
            // Output files
            $($out:expr),* =>
            // Command
            $cmd:ident
            // Input files
            ($($in:expr),*$(,)?)
            ;
        )*
    ) => {
        {
            #[allow(unused)]
            struct MockContext {
                graph: n2o4::graph::BuildGraph,
                $($id: n2o4::graph::BuildId,)*
            }

            let mut __gb = n2o4::graph::GraphBuilder::new();
            $(
                let __outs = vec![$(__gb.add_file($out)),*];
                let __ins = vec![$(__gb.add_file($in)),*];
                let __build = n2o4::graph::BuildNode {
                    command: n2o4::graph::BuildMethod::SubCommand(n2o4::graph::BuildCommand {
                        executable: std::path::PathBuf::from(stringify!($cmd)),
                        args: vec![],
                    }),
                    ins: __ins,
                    outs: __outs,
                    restat: false,
                };
                let __build_id = __gb.add_build(__build);
                let $id = __build_id;
                $(
                    $(
                        __gb.add_build_dep(__build_id, $dep);
                    )*
                )?
            )*
            let graph = __gb.build().unwrap();
            MockContext { graph, $($id,)* }
        }
    };
}

// 0) No-op run (no nodes); assert no errors
#[test]
fn test_nothing() {
    let cfg = ExecConfig::default();
    let cx = mock_graph! {};
    let db = Box::new(InMemoryDb::default());
    let world = MockWorld::new();
    let mut executor = Executor::with_world(&cfg, &cx.graph, db, &world, &());
    executor.run().unwrap();
}

// 1) Single node: Outdated -> Succeeded; assert exec log and file info written
#[test]
fn test_single_node_outdated_succeeded() {
    // Build graph: A(in.txt -> out.txt)
    let cx = mock_graph! {
        a: "out.txt" => A("in.txt");
    };

    // World and DB
    let world = MockWorld::new();
    world.touch_file("in.txt"); // ensure input exists

    let db = InMemoryDb::default();
    let db_read = db.clone();
    let db_box: Box<dyn n2o4::db::ExecDb> = Box::new(db);

    let cfg = ExecConfig::default();
    let mut exec = Executor::with_world(&cfg, &cx.graph, db_box, &world, &());
    exec.want([cx.a]);
    exec.run().unwrap();

    let labels = drain_exec_labels(&world);
    assert_eq!(labels, vec!["A"]);

    let rd = db_read.begin_read();
    assert!(rd.get_file_info("out.txt".as_ref()).is_some());
}

// 2) Single node: Outdated -> Failed; assert exec log and file info invalidated
#[test]
fn test_single_node_outdated_failed() {
    // Build graph: A(in.txt -> out.txt)
    let cx = mock_graph! {
        a: "out.txt" => A("in.txt");
    };

    // World and DB
    let world = MockWorld::new();
    world.touch_file("in.txt");
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
    let mut exec = Executor::with_world(&cfg, &cx.graph, db_box, &world, &());
    exec.want([cx.a]);
    exec.run().unwrap();

    let labels = drain_exec_labels(&world);
    assert_eq!(labels, vec!["A"]);

    let rd = db_read.begin_read();
    // Failed build should invalidate/avoid any file info for the output
    assert!(rd.get_file_info("out.txt".as_ref()).is_none());
}

// 3) Single node: UpToDate on second run (no execution)
#[test]
fn test_single_node_up_to_date() {
    // Build graph: A(in.txt -> out.txt)
    let cx = mock_graph! {
        a: "out.txt" => A("in.txt");
    };

    let world = MockWorld::new();
    world.touch_file("in.txt");

    let db = InMemoryDb::default();
    let db_read = db.clone();

    // First run to populate DB
    {
        let db_box: Box<dyn n2o4::db::ExecDb> = Box::new(db.clone());
        let cfg = ExecConfig::default();
        let mut exec = Executor::with_world(&cfg, &cx.graph, db_box, &world, &());
        exec.want([cx.a]);
        exec.run().unwrap();
        // Drain first-run log to isolate second-run behavior
        let _ = drain_exec_labels(&world);
    }

    // Second run should be UpToDate and not execute the command
    {
        let db_box: Box<dyn n2o4::db::ExecDb> = Box::new(db_read.clone());
        let cfg = ExecConfig::default();
        let mut exec = Executor::with_world(&cfg, &cx.graph, db_box, &world, &());
        exec.want([cx.a]);
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
    assert!(rd.get_file_info("out.txt".as_ref()).is_some());
}

// 4) Linear , dependency: A -> B success path
#[test]
fn test_linear_dependency_success() {
    // A(a.in -> a.out), B(a.out -> b.out), with edge B , depends on A
    let cx = mock_graph! {
        a: "a.out" => A("a.in");
        b, dep(a): "b.out" => B("a.out");
    };

    let world = MockWorld::new();
    world.touch_file("a.in");
    // Ensure , dependent input exists so B won't be Missing when scheduled
    world.touch_file("a.out");

    let db = InMemoryDb::default();
    let db_read = db.clone();
    let db_box: Box<dyn n2o4::db::ExecDb> = Box::new(db);

    let cfg = ExecConfig::default();
    let mut exec = Executor::with_world(&cfg, &cx.graph, db_box, &world, &());
    exec.want([cx.b]); // Want B should pull in A via , dependency
    exec.run().unwrap();

    let labels = drain_exec_labels(&world);
    assert_eq!(labels.len(), 2);
    assert!(labels.contains(&"A".to_string()));
    assert!(labels.contains(&"B".to_string()));

    let rd = db_read.begin_read();
    assert!(rd.get_file_info("b.out".as_ref()).is_some());
}

// 5) Failure propagation: A Failed -> B Skipped (B not executed)
#[test]
fn test_dependency_failure_propagation_skipped() {
    // A(a.in -> a.out), B(a.out -> b.out), B , depends on A
    let cx = mock_graph! {
        a: "a.out" => A("a.in");
        b, dep(a): "b.out" => B("a.out");
    };

    let world = MockWorld::new();
    world.touch_file("a.in");
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
    let mut exec = Executor::with_world(&cfg, &cx.graph, db_box, &world, &());
    exec.want([cx.b]);
    exec.run().unwrap();

    let labels = drain_exec_labels(&world);
    // Only A should execute; B is skipped
    assert_eq!(labels, vec!["A"]);

    let rd = db_read.begin_read();
    // Neither A nor B should have output info (A failed -> invalidated; B skipped)
    assert!(rd.get_file_info("a.out".as_ref()).is_none());
    assert!(rd.get_file_info("b.out".as_ref()).is_none());
}

// 6) Multiple inputs gate: B executes only after A and C succeed
#[test]
fn test_multi_input_gatekeeping() {
    // A(a.in -> a.out), C(c.in -> c.out), B([a.out, c.out] -> b.out)
    let cx = mock_graph! {
        a: "a.out" => A("a.in");
        c: "c.out" => C("c.in");
        b, dep(a, c): "b.out" => B("a.out", "c.out");
    };

    let world = MockWorld::new();
    world.touch_file("a.in");
    world.touch_file("c.in");
    // Ensure , dependent inputs exist so B won't be Missing when scheduled
    world.touch_file("a.out");
    world.touch_file("c.out");

    let db = InMemoryDb::default();
    let db_read = db.clone();
    let db_box: Box<dyn n2o4::db::ExecDb> = Box::new(db);

    let cfg = ExecConfig::default();
    let mut exec = Executor::with_world(&cfg, &cx.graph, db_box, &world, &());
    exec.want([cx.b]);
    exec.run().unwrap();

    let labels = drain_exec_labels(&world);
    assert_eq!(labels.len(), 3);
    assert!(labels.contains(&"A".to_string()));
    assert!(labels.contains(&"C".to_string()));
    assert!(labels.contains(&"B".to_string()));

    let rd = db_read.begin_read();
    assert!(rd.get_file_info("b.out".as_ref()).is_some());
}

// 7) Skipped chain propagation: A Failed -> B Skipped -> C Skipped (B , depends on A, C , depends on B)
#[test]
fn test_skipped_chain_propagation() {
    // A(a.in -> a.out) -> B(a.out -> b.out) -> C(b.out -> c.out)
    let cx = mock_graph! {
        a: "a.out" => A("a.in");
        b, dep(a): "b.out" => B("a.out");
        c, dep(b): "c.out" => C("b.out");
    };

    let world = MockWorld::new();
    world.touch_file("a.in");
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
    let mut exec = Executor::with_world(&cfg, &cx.graph, db_box, &world, &());
    exec.want([cx.c]); // Want C pulls in B and A
    exec.run().unwrap();

    let labels = drain_exec_labels(&world);
    // Only A executed; B and C should be skipped
    assert_eq!(labels, vec!["A"]);

    let rd = db_read.begin_read();
    assert!(rd.get_file_info("a.out".as_ref()).is_none());
    assert!(rd.get_file_info("b.out".as_ref()).is_none());
    assert!(rd.get_file_info("c.out".as_ref()).is_none());
}

// 8) Optional: parallelism=1 with two leaves; sequential execution (no strict order asserted)
#[test]
fn test_parallelism_one_two_leaves() {
    // D(d.in -> d.out), E(e.in -> e.out) 无依赖
    let cx = mock_graph! {
        d: "d.out" => D("d.in");
        e: "e.out" => E("e.in");
    };

    let world = MockWorld::new();
    world.touch_file("d.in");
    world.touch_file("e.in");

    let db = InMemoryDb::default();
    let db_box: Box<dyn n2o4::db::ExecDb> = Box::new(db);

    let cfg = ExecConfig { parallelism: 1 };

    let mut exec = Executor::with_world(&cfg, &cx.graph, db_box, &world, &());
    exec.want([cx.d, cx.e]);
    exec.run().unwrap();

    let labels = drain_exec_labels(&world);
    assert_eq!(labels.len(), 2);
    assert!(labels.contains(&"D".to_string()));
    assert!(labels.contains(&"E".to_string()));
}
