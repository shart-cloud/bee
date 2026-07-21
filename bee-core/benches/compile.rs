//! Benchmark: policy compilation must stay under 50ms for ≤100 path + ≤50 net rules (SC-006/NFR-002).

use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;

use bee_core::{CompileError, Policy, Resolver};
use criterion::{criterion_group, criterion_main, Criterion};

struct BenchResolver;

impl Resolver for BenchResolver {
    fn project_root(&self) -> &str {
        "/home/u/project"
    }
    fn home(&self) -> &str {
        "/home/u"
    }
    fn resolve_exec(&self, name: &str) -> Result<PathBuf, CompileError> {
        Ok(PathBuf::from(format!("/usr/bin/{name}")))
    }
    fn resolve_host(&self, _host: &str) -> Result<Vec<IpAddr>, CompileError> {
        Ok(vec![IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34))])
    }
}

fn big_policy() -> Policy {
    let mut toml = String::from("[policy]\nname = \"bench\"\n\n[policy.filesystem]\n");
    for i in 0..100 {
        toml.push_str(&format!("\"/home/u/project/dir{i}\" = \"write\"\n"));
    }
    toml.push_str(
        "\n[policy.exec]\nallow = [\"cargo\", \"rustc\", \"cc\"]\n\n[policy.network]\nallow = [",
    );
    for i in 0..50 {
        if i > 0 {
            toml.push_str(", ");
        }
        toml.push_str(&format!("\"host{i}.example.com:443\""));
    }
    toml.push_str("]\n");
    Policy::from_toml(&toml).expect("valid policy")
}

fn bench_compile(c: &mut Criterion) {
    let policy = big_policy();
    let resolver = BenchResolver;
    c.bench_function("compile_100_paths_50_nets", |b| {
        b.iter(|| policy.compile(&resolver).unwrap())
    });
}

criterion_group!(benches, bench_compile);
criterion_main!(benches);
