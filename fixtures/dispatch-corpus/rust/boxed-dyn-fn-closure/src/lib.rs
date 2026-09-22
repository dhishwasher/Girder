pub fn target() -> i32 {
    42
}

pub struct Runner {
    pub callback: Box<dyn Fn() -> i32>,
}

impl Runner {
    pub fn run(&self) -> i32 {
        (self.callback)()
    }
}

#[test]
fn test_boxed_closure() {
    let r = Runner {
        callback: Box::new(target),
    };
    assert_eq!(r.run(), 42);
}
