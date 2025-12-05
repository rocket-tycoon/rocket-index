use criterion::{black_box, criterion_group, criterion_main, Criterion};
use rocketindex::languages::fsharp::FSharpResolver;
use rocketindex::resolve::SymbolResolver;
use rocketindex::{CodeIndex, Location, Symbol, SymbolKind, Visibility};
use std::path::PathBuf;

fn create_large_index(size: usize) -> CodeIndex {
    let mut index = CodeIndex::new();

    for i in 0..size {
        let name = format!("symbol_{}", i);
        let qualified = format!("MyApp.Module_{}.{}", i / 100, name);
        let file = format!("src/Module_{}.fs", i / 100);

        index.add_symbol(Symbol::new(
            name,
            qualified,
            SymbolKind::Function,
            Location::new(PathBuf::from(file), 1, 1),
            Visibility::Public,
            "fsharp".to_string(),
        ));
    }

    // Add some opens
    for i in 0..size / 100 {
        let file = PathBuf::from(format!("src/Module_{}.fs", i));
        index.add_open(file, "MyApp.Common".to_string());
    }

    index
}

fn benchmark_resolve(c: &mut Criterion) {
    let index = create_large_index(10000);
    let resolver = FSharpResolver;
    let from_file = PathBuf::from("src/Module_0.fs");

    c.bench_function("resolve_exact", |b| {
        b.iter(|| {
            resolver.resolve(
                black_box(&index),
                black_box("MyApp.Module_50.symbol_5000"),
                black_box(&from_file),
            )
        })
    });

    c.bench_function("resolve_unqualified", |b| {
        b.iter(|| {
            resolver.resolve(
                black_box(&index),
                black_box("symbol_5"), // Should be in same file/module
                black_box(&from_file),
            )
        })
    });
}

criterion_group!(benches, benchmark_resolve);
criterion_main!(benches);
