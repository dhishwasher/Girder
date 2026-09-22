pub trait Speaker {
    fn speak(&self) -> &str;
}

pub struct Dog;

impl Speaker for Dog {
    fn speak(&self) -> &str {
        "woof"
    }
}

pub struct Cat;

impl Speaker for Cat {
    fn speak(&self) -> &str {
        "meow"
    }
}

pub fn announce<S: Speaker>(s: &S) -> &str {
    s.speak()
}

#[test]
fn test_dog() {
    let d = Dog;
    assert_eq!(announce(&d), "woof");
}
