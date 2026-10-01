pub fn only_via_a() -> i32 {
    1
}

pub fn only_via_b() -> i32 {
    2
}

pub struct S;

pub trait A {
    fn go(&self) -> i32;
}

pub trait B {
    fn go(&self) -> i32;
}

impl A for S {
    fn go(&self) -> i32 {
        only_via_a() + only_via_a()
    }
}

impl B for S {
    fn go(&self) -> i32 {
        only_via_b()
    }
}

#[test]
fn calls_a() {
    assert_eq!(A::go(&S), 1);
}

#[test]
fn calls_b() {
    assert_eq!(B::go(&S), 2);
}
