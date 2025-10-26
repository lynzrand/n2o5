use std::os::raw::c_int;

unsafe extern "C" {
    fn add_numbers(a: c_int, b: c_int) -> c_int;
}

fn main() {
    let a: c_int = 2;
    let b: c_int = 3;
    let sum = unsafe { add_numbers(a, b) };
    println!("{a} + {b} = {sum}");
    assert_eq!(sum, 5);
}
