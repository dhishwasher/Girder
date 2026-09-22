pub fn target() -> i32 {
    42
}

pub fn run(f: fn() -> i32) -> i32 {
    f()
}

#[test]
fn test_via_pointer() {
    let p: fn() -> i32 = target;
    assert_eq!(run(p), 42);
}
