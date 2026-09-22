pub fn target(x: i32) -> i32 {
    x * 2
}

pub fn run(values: Vec<i32>) -> Vec<i32> {
    values.into_iter().map(|x| target(x)).collect()
}

#[test]
fn test_via_closure() {
    assert_eq!(run(vec![1, 2, 3]), vec![2, 4, 6]);
}
