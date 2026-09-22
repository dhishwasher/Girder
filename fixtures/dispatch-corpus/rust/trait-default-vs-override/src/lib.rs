pub trait Describable {
    fn describe(&self) -> String {
        "default description".to_string()
    }
}

pub struct Widget;

impl Describable for Widget {}

pub struct Gadget;

impl Describable for Gadget {
    fn describe(&self) -> String {
        "custom gadget description".to_string()
    }
}

pub fn render(d: &dyn Describable) -> String {
    d.describe()
}

#[test]
fn test_widget_default() {
    let w = Widget;
    assert_eq!(render(&w), "default description");
}

#[test]
fn test_gadget_override() {
    let g = Gadget;
    assert_eq!(render(&g), "custom gadget description");
}
