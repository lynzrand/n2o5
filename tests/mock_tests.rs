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

// Helper functions

fn declare_db() -> (InMemoryDb, Box<dyn ExecDb>) {
    let db = InMemoryDb::default();
    let db_box: Box<dyn ExecDb> = Box::new(db.clone());
    (db, db_box)
}

fn run_graph(
    world: &MockWorld,
    graph: &n2o4::graph::BuildGraph,
    cfg: ExecConfig,
    db: Box<dyn ExecDb>,
    want: impl IntoIterator<Item = n2o4::graph::BuildId>,
) -> Vec<String> {
    let mut exec = Executor::with_world(&cfg, graph, db, world, &());
    exec.want(want);
    exec.run().unwrap();
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

fn touch_all(world: &MockWorld, files: &[&str]) {
    for f in files {
        world.touch_file(f);
    }
}

fn set_fail_on(world: &MockWorld, exec_name: &str) {
    let name = exec_name.to_string();
    world.set_callback(Box::new(move |_, method| {
        if let BuildMethod::SubCommand(cmd) = method
            && cmd.executable == Path::new(&name)
        {
            return Ok(BuildStatusKind::Failed);
        }
        Ok(BuildStatusKind::Succeeded)
    }));
}

fn assert_db_has(db: &InMemoryDb, path: &str) {
    let rd = db.begin_read();
    assert!(rd.get_file_info(Path::new(path)).is_some());
}

fn assert_db_missing(db: &InMemoryDb, path: &str) {
    let rd = db.begin_read();
    assert!(rd.get_file_info(Path::new(path)).is_none());
}

fn assert_log_include(log: &[String], expected: &[&str]) {
    for e in expected {
        assert!(
            log.contains(&e.to_string()),
            "Expected log to include {}. Got {:?}",
            e,
            log
        );
    }
}

fn assert_order(log: &[String], before: &str, after: &str) {
    let b = log
        .iter()
        .position(|l| l == before)
        .unwrap_or_else(|| panic!("Expected '{}' in log {:?}", before, log));
    let a = log
        .iter()
        .position(|l| l == after)
        .unwrap_or_else(|| panic!("Expected '{}' in log {:?}", after, log));
    assert!(
        b < a,
        "Expected '{}' to execute before '{}'. Got {:?}",
        before,
        after,
        log
    );
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
    let cx = mock_graph! {
        a: "out.txt" => A("in.txt");
    };

    let world = MockWorld::new();
    touch_all(&world, &["in.txt"]);

    let (db_read, db_box) = declare_db();

    let log = run_graph(&world, &cx.graph, ExecConfig::default(), db_box, [cx.a]);
    assert_eq!(log, vec!["A"]);

    assert_db_has(&db_read, "out.txt");
}

// 2) Single node: Outdated -> Failed; assert exec log and file info invalidated
#[test]
fn test_single_node_outdated_failed() {
    let cx = mock_graph! {
        a: "out.txt" => A("in.txt");
    };

    let world = MockWorld::new();
    touch_all(&world, &["in.txt"]);
    set_fail_on(&world, "A");

    let (db_read, db_box) = declare_db();

    let log = run_graph(&world, &cx.graph, ExecConfig::default(), db_box, [cx.a]);
    assert_eq!(log, vec!["A"]);

    assert_db_missing(&db_read, "out.txt");
}

// 3) Single node: UpToDate on second run (no execution)
#[test]
fn test_single_node_up_to_date() {
    let cx = mock_graph! {
        a: "out.txt" => A("in.txt");
    };

    let world = MockWorld::new();
    touch_all(&world, &["in.txt"]);

    let (db_read, db_box) = declare_db();

    // First run to populate DB
    let _ = run_graph(&world, &cx.graph, ExecConfig::default(), db_box, [cx.a]);

    // Second run should be UpToDate and not execute the command
    let db_box2: Box<dyn n2o4::db::ExecDb> = Box::new(db_read.clone());
    let log = run_graph(&world, &cx.graph, ExecConfig::default(), db_box2, [cx.a]);
    assert!(
        log.is_empty(),
        "Expected no execution on UpToDate, got {:?}",
        log
    );

    // File info should still exist
    assert_db_has(&db_read, "out.txt");
}

// 4) Linear , dependency: A -> B success path
#[test]
fn test_linear_dependency_success() {
    let cx = mock_graph! {
        a: "a.out" => A("a.in");
        b, dep(a): "b.out" => B("a.out");
    };

    let world = MockWorld::new();
    touch_all(&world, &["a.in", "a.out"]);

    let (db_read, db_box) = declare_db();

    let log = run_graph(&world, &cx.graph, ExecConfig::default(), db_box, [cx.b]);

    assert_eq!(log.len(), 2);
    assert_order(&log, "A", "B");

    assert_db_has(&db_read, "b.out");
}

// 5) Failure propagation: A Failed -> B Skipped (B not executed)
#[test]
fn test_dependency_failure_propagation_skipped() {
    let cx = mock_graph! {
        a: "a.out" => A("a.in");
        b, dep(a): "b.out" => B("a.out");
    };

    let world = MockWorld::new();
    touch_all(&world, &["a.in"]);
    set_fail_on(&world, "A");

    let (db_read, db_box) = declare_db();

    let log = run_graph(&world, &cx.graph, ExecConfig::default(), db_box, [cx.b]);
    assert_eq!(log, vec!["A"]);

    assert_db_missing(&db_read, "a.out");
    assert_db_missing(&db_read, "b.out");
}

// 6) Multiple inputs gate: B executes only after A and C succeed
#[test]
fn test_multi_input_gatekeeping() {
    let cx = mock_graph! {
        a: "a.out" => A("a.in");
        c: "c.out" => C("c.in");
        b, dep(a, c): "b.out" => B("a.out", "c.out");
    };

    let world = MockWorld::new();
    touch_all(&world, &["a.in", "c.in", "a.out", "c.out"]);

    let (db_read, db_box) = declare_db();

    let log = run_graph(&world, &cx.graph, ExecConfig::default(), db_box, [cx.b]);
    assert_eq!(log.len(), 3);
    assert_order(&log, "A", "B");
    assert_order(&log, "C", "B");

    assert_db_has(&db_read, "b.out");
}

// 7) Skipped chain propagation: A Failed -> B Skipped -> C Skipped (B , depends on A, C , depends on B)
#[test]
fn test_skipped_chain_propagation() {
    let cx = mock_graph! {
        a: "a.out" => A("a.in");
        b, dep(a): "b.out" => B("a.out");
        c, dep(b): "c.out" => C("b.out");
    };

    let world = MockWorld::new();
    touch_all(&world, &["a.in"]);
    set_fail_on(&world, "A");

    let (db_read, db_box) = declare_db();

    let log = run_graph(&world, &cx.graph, ExecConfig::default(), db_box, [cx.c]);
    assert_eq!(log, vec!["A"]);

    assert_db_missing(&db_read, "a.out");
    assert_db_missing(&db_read, "b.out");
    assert_db_missing(&db_read, "c.out");
}

// 8) Optional: parallelism=1 with two leaves; sequential execution (no strict order asserted)
#[test]
fn test_parallelism_one_two_leaves() {
    let cx = mock_graph! {
        d: "d.out" => D("d.in");
        e: "e.out" => E("e.in");
    };

    let world = MockWorld::new();
    touch_all(&world, &["d.in", "e.in"]);

    let (_db_read, db_box) = declare_db();

    let log = run_graph(
        &world,
        &cx.graph,
        ExecConfig { parallelism: 1 },
        db_box,
        [cx.d, cx.e],
    );
    assert_eq!(log.len(), 2);
    assert_log_include(&log, &["D", "E"]);
}

#[test]
fn test_failure_midway_propagation() {
    let cx = mock_graph! {
        a: "a.out" => A("a.in");
        b, dep(a): "b.out" => B("a.out");
        c, dep(b): "c.out" => C("b.out");
    };

    let world = MockWorld::new();
    touch_all(&world, &["a.in"]);
    set_fail_on(&world, "B");

    let (db_read, db_box) = declare_db();

    let log = run_graph(&world, &cx.graph, ExecConfig::default(), db_box, [cx.c]);
    assert_eq!(log, vec!["A", "B"]);
    assert_db_has(&db_read, "a.out");
}

#[test]
fn test_up_to_date() {
    let cx = mock_graph! {
        a: "a.out" => A("a.in");
        b, dep(a): "b.out" => B("a.out");
    };

    let world = MockWorld::new();
    touch_all(&world, &["a.in"]);

    let (db_read, db_box) = declare_db();

    // First run to populate DB
    let _ = run_graph(&world, &cx.graph, ExecConfig::default(), db_box, [cx.b]);

    // Second run should be UpToDate and not execute the command
    let db_box2: Box<dyn n2o4::db::ExecDb> = Box::new(db_read.clone());
    let log = run_graph(&world, &cx.graph, ExecConfig::default(), db_box2, [cx.b]);
    assert!(
        log.is_empty(),
        "Expected no execution on UpToDate, got {:?}",
        log
    );

    // File info should still exist
    assert_db_has(&db_read, "b.out");
}

fn set_fail_on_any(world: &MockWorld, exec_names: &[&str]) {
    let names: Vec<String> = exec_names.iter().map(|s| s.to_string()).collect();
    world.set_callback(Box::new(move |_, method| {
        if let BuildMethod::SubCommand(cmd) = method
            && names.iter().any(|n| cmd.executable == Path::new(n))
        {
            return Ok(BuildStatusKind::Failed);
        }
        Ok(BuildStatusKind::Succeeded)
    }));
}

#[test]
fn test_two_dependency_failures_skip_consumer() {
    let cx = mock_graph! {
        a: "a.out" => A("a.in");
        b: "b.out" => B("b.in");
        c, dep(a, b): "c.out" => C("a.out", "b.out");
    };

    let world = MockWorld::new();
    touch_all(&world, &["a.in", "b.in"]);
    set_fail_on_any(&world, &["A", "B"]);

    let (db_read, db_box) = declare_db();

    // Both A and B should have failed, C skipped
    // No error should be raised
    let _log = run_graph(&world, &cx.graph, ExecConfig::default(), db_box, [cx.c]);

    assert_db_missing(&db_read, "a.out");
    assert_db_missing(&db_read, "b.out");
    assert_db_missing(&db_read, "c.out");
}

#[test]
fn test_touch_input_after_first_build_triggers_rebuild() {
    let cx = mock_graph! {
        a: "out.txt" => A("in.txt");
    };

    let world = MockWorld::new();
    touch_all(&world, &["in.txt"]);

    let (db_read, db_box) = declare_db();

    // First run to populate DB
    let log = run_graph(&world, &cx.graph, ExecConfig::default(), db_box, [cx.a]);
    assert_db_has(&db_read, "out.txt");
    assert_eq!(log, vec!["A"]);

    // Touch input after the first build
    world.touch_file("in.txt");

    // Second run should rebuild due to input mtime > last_start
    let db_box2: Box<dyn n2o4::db::ExecDb> = Box::new(db_read.clone());
    let log = run_graph(&world, &cx.graph, ExecConfig::default(), db_box2, [cx.a]);
    assert_eq!(log, vec!["A"]);

    // File info should still exist
    assert_db_has(&db_read, "out.txt");
}

#[test]
fn test_change_command_then_change_back_reuses_same_db() {
    // Initial build with A
    let cx1 = mock_graph! {
        a: "out.txt" => A("in.txt");
    };

    let world = MockWorld::new();
    touch_all(&world, &["in.txt"]);

    let (db_read, db_box) = declare_db();
    let _ = run_graph(&world, &cx1.graph, ExecConfig::default(), db_box, [cx1.a]);

    // Change command to X, same inputs/outputs, reuse the same DB
    let cx2 = mock_graph! {
        a: "out.txt" => X("in.txt");
    };
    let db_box2: Box<dyn n2o4::db::ExecDb> = Box::new(db_read.clone());
    let log2 = run_graph(&world, &cx2.graph, ExecConfig::default(), db_box2, [cx2.a]);
    assert_eq!(log2, vec!["X"]);
    assert_db_has(&db_read, "out.txt");

    // Change command back to A and build again with the same DB
    let cx3 = mock_graph! {
        a: "out.txt" => A("in.txt");
    };
    let db_box3: Box<dyn n2o4::db::ExecDb> = Box::new(db_read.clone());
    let log3 = run_graph(&world, &cx3.graph, ExecConfig::default(), db_box3, [cx3.a]);
    assert_eq!(log3, vec!["A"]);
    assert_db_has(&db_read, "out.txt");

    // Another build with A should be UpToDate (no execution)
    let cx4 = mock_graph! {
        a: "out.txt" => A("in.txt");
    };
    let db_box4: Box<dyn n2o4::db::ExecDb> = Box::new(db_read.clone());
    let log4 = run_graph(&world, &cx4.graph, ExecConfig::default(), db_box4, [cx3.a]);
    assert_eq!(log4.len(), 0);
    assert_db_has(&db_read, "out.txt");
}

#[test]
fn test_remove_output_file_after_successful_build() {
    let cx = mock_graph! {
        a: "out.txt" => A("in.txt");
    };

    let world = MockWorld::new();
    touch_all(&world, &["in.txt"]);

    let (db_read, db_box) = declare_db();

    // First run to populate DB
    let _ = run_graph(&world, &cx.graph, ExecConfig::default(), db_box, [cx.a]);
    assert_db_has(&db_read, "out.txt");

    // Simulate removing the output file from the world
    world.remove_file("out.txt");

    let db_box2: Box<dyn n2o4::db::ExecDb> = Box::new(db_read.clone());
    let log = run_graph(&world, &cx.graph, ExecConfig::default(), db_box2, [cx.a]);
    // Command should re-execute to regenerate the missing output
    assert_eq!(log, vec!["A"]);

    // DB still tracks the file info
    assert_db_has(&db_read, "out.txt");
}

#[test]
fn test_nonexisting_input_file_fails_without_execution() {
    let cx = mock_graph! {
        a: "out.txt" => A("missing.in");
    };

    let world = MockWorld::new();

    let (db_read, db_box) = declare_db();

    let log = run_graph(&world, &cx.graph, ExecConfig::default(), db_box, [cx.a]);
    assert!(
        log.is_empty(),
        "Expected no execution when input file is missing, got {:?}",
        log
    );

    assert_db_missing(&db_read, "out.txt");
}
