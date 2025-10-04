use expect_test::ExpectFile;
use n2o5_cli::ninja::parser::{ParseSource, parse};

fn snapshot(s: &str, exp: ExpectFile) {
    let source = ParseSource::new_in_memory(s);
    let parsed = parse(&source, source.main_file());
    exp.assert_debug_eq(&parsed);
}

macro_rules! snapshot_files {
    ($($filename:ident),*$(,)?) => {
        $(
            #[test]
            fn $filename() {
                let s = include_str!(concat!(
                    "./parser_snapshots/",
                    stringify!($filename),
                    ".ninja"
                ));
                let exp = expect_test::expect_file![concat!(
                    "./parser_snapshots/",
                    stringify!($filename),
                    ".snap"
                )];
                snapshot(s, exp);
            }
        )*
    };
}

snapshot_files!(depfile, msvc, var_expansion_1, var_expansion_2);
