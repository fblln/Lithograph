pub struct Broken {
    pub field: i32,

pub fn unclosed_fn(x: i32) -> i32 {
    if x > 0 {
        return x;
    // missing closing braces
