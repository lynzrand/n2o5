use std::{any::Any, env, fs, io, path::PathBuf};

use cc::Build as CcBuild;
use n2o5::{
    db::dumb::DumbDb,
    exec::{ExecConfig, Executor},
    graph::{BuildMethod, BuildNode, GraphBuilder},
    progress::noop::NOOP_PROGRESS,
    world::LOCAL_WORLD,
};

type DynError = Box<dyn std::error::Error + Send + Sync>;

const DEMO_SOURCE: &str = r#"
#include <stdint.h>

int add_numbers(int a, int b) {
    return a + b;
}
"#;

struct BuildContext {
    out_dir: PathBuf,
    generated_c: PathBuf,
    static_lib: PathBuf,
    lib_name: String,
}

struct BuildConfig {
    generated_c: PathBuf,
    out_dir: PathBuf,
    lib_name: String,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), DynError> {
    let config = BuildConfig::from_env()?;

    fs::create_dir_all(&config.out_dir)?;
    if let Some(parent) = config.generated_c.parent() {
        fs::create_dir_all(parent)?;
    }

    let static_lib = config.out_dir.join(format!("lib{}.a", config.lib_name));

    let ctx = BuildContext {
        out_dir: config.out_dir.clone(),
        generated_c: config.generated_c.clone(),
        static_lib: static_lib.clone(),
        lib_name: config.lib_name.clone(),
    };

    let mut builder = GraphBuilder::new();
    let generated_c_id = builder.add_file_owned(config.generated_c.clone());
    let static_lib_id = builder.add_file_owned(static_lib.clone());

    let generate_node = builder.add_build(BuildNode {
        command: BuildMethod::Callback("generate-demo-c".into(), Box::new(generate_demo_source)),
        ins: vec![],
        outs: vec![generated_c_id],
        description: Some("emit demo C source".into()),
    });

    let compile_node = builder.add_build(BuildNode {
        command: BuildMethod::Callback("compile-demo-lib".into(), Box::new(compile_demo_library)),
        ins: vec![generated_c_id],
        outs: vec![static_lib_id],
        description: Some("compile static library with cc".into()),
    });

    builder.add_build_dep(compile_node, generate_node);
    let graph = builder.build()?;

    let db_path = ctx.out_dir.join("n2o5-dumb-db.bin");
    let exec_cfg = ExecConfig::default();

    println!("cargo:warning=Running n2o5 demo graph to populate cache");
    let db = DumbDb::new(&db_path)?;
    {
        let mut executor =
            Executor::with_world(&exec_cfg, &graph, &db, &LOCAL_WORLD, &NOOP_PROGRESS, &ctx);
        executor.want([compile_node]);
        executor.run()?;
    }
    drop(db);

    println!("cargo:warning=Running n2o5 demo graph again to show cache hit");
    let db = DumbDb::new(&db_path)?;
    {
        let mut executor =
            Executor::with_world(&exec_cfg, &graph, &db, &LOCAL_WORLD, &NOOP_PROGRESS, &ctx);
        executor.want([compile_node]);
        executor.run()?;
    }

    println!("cargo:rustc-link-search=native={}", ctx.out_dir.display());
    println!("cargo:rustc-link-lib=static={}", ctx.lib_name);

    Ok(())
}

impl BuildConfig {
    fn from_env() -> Result<Self, DynError> {
        let out_dir = env::var_os("OUT_DIR")
            .ok_or_else(|| arg_error("OUT_DIR not set; this example must run under Cargo"))?;
        let out_dir = PathBuf::from(out_dir);

        let generated_c = out_dir.join("n2o5_demo.c");

        let lib_name = env::var("CARGO_PKG_NAME").unwrap_or_else(|_| "n2o5_demo".into());

        Ok(Self {
            generated_c,
            out_dir,
            lib_name,
        })
    }
}

fn build_ctx(state: &dyn Any) -> &BuildContext {
    state
        .downcast_ref::<BuildContext>()
        .expect("callbacks receive the BuildContext state")
}

fn generate_demo_source(state: &dyn Any) -> Result<(), DynError> {
    let ctx = build_ctx(state);
    if let Some(parent) = ctx.generated_c.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::write(&ctx.generated_c, DEMO_SOURCE.as_bytes())?;
    println!("cargo:warning=wrote {}", ctx.generated_c.display());
    Ok(())
}

fn compile_demo_library(state: &dyn Any) -> Result<(), DynError> {
    let ctx = build_ctx(state);
    println!(
        "cargo:warning=invoking cc to produce {}",
        ctx.static_lib.display()
    );

    let mut build = CcBuild::new();
    build.file(&ctx.generated_c);
    build.out_dir(&ctx.out_dir);
    build
        .try_compile(&ctx.lib_name)
        .map_err(|err| -> DynError { Box::new(err) })?;

    Ok(())
}

fn arg_error(message: impl Into<String>) -> DynError {
    Box::new(io::Error::new(io::ErrorKind::InvalidInput, message.into()))
}
