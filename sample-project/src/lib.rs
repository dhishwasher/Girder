//! Tiny sample crate that AetherForge loads into its semantic graph.
//! The GUI's "Code Projection" panel mirrors this file; the agent swarm adds a
//! `multiply` function to it on demand.

pub struct Point {
    pub x: i64,
    pub y: i64,
}

pub fn add(a: i64, b: i64) -> i64 {
    a + b
}

pub fn sum_list(xs: &[i64]) -> i64 {
    let mut total = 0;
    for x in xs {
        total = add(total, *x);
    }
    total
}

pub fn main() {
    let r = sum_list(&[1, 2, 3]);
    println!("{r}");
}
