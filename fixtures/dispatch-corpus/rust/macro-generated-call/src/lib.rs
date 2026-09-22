pub fn target() -> i32 {
    42
}

macro_rules! call_target {
    () => {
        target()
    };
}

pub fn run() -> i32 {
    call_target!()
}

#[test]
fn test_via_macro() {
    assert_eq!(run(), 42);
}
