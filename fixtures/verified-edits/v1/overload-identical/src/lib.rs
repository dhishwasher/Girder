pub trait Render {
    fn render(&self) -> String;
}

pub struct Widget;

impl Widget {
    fn render(&self) -> String {
        String::from("same")
    }
}

impl Render for Widget {
    fn render(&self) -> String {
        String::from("same")
    }
}

pub fn draw(w: &Widget) -> String {
    w.render()
}

pub fn draw_via_trait<T: Render>(t: &T) -> String {
    Render::render(t)
}

pub fn helper() -> usize {
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_caller_reaches_its_overload() {
        assert_eq!(draw(&Widget), draw_via_trait(&Widget));
    }
}
