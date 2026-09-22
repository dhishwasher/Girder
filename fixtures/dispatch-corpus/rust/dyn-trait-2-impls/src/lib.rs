pub trait Greeter {
    fn greet(&self) -> &str;
}

pub struct English;

impl Greeter for English {
    fn greet(&self) -> &str {
        "hello"
    }
}

pub struct French;

impl Greeter for French {
    fn greet(&self) -> &str {
        "bonjour"
    }
}

pub fn dispatch(g: &dyn Greeter) -> &str {
    g.greet()
}

#[test]
fn test_via_english() {
    let e = English;
    assert_eq!(dispatch(&e), "hello");
}

#[test]
fn test_via_french() {
    let f = French;
    assert_eq!(dispatch(&f), "bonjour");
}
