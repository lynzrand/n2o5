use n2o4::{
    db::in_memory::InMemoryDb,
    exec::{ExecConfig, Executor},
    graph::BuildGraph,
};

use crate::mock::MockWorld;

mod mock;

#[test]
fn test_nothing() {
    let cfg = ExecConfig::default();
    let graph = BuildGraph::default();
    let db = Box::new(InMemoryDb::default());
    let world = MockWorld::new();
    let mut executor = Executor::with_world(&cfg, &graph, db, &world, &());

    executor.run().unwrap();
}
