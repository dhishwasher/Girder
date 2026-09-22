use std::ops::Add;

pub struct Meters(pub i32);

impl Add for Meters {
    type Output = Meters;
    fn add(self, other: Meters) -> Meters {
        Meters(self.0 + other.0)
    }
}

#[test]
fn test_add_meters() {
    let total = Meters(2) + Meters(3);
    assert_eq!(total.0, 5);
}
