//! Minimal Rust fixture for testing

pub mod utils {
    pub fn helper() -> i32 {
        42
    }
}

pub fn main_function() {
    let x = utils::helper();
    println!("{}", x);
}

pub fn caller_a() {
    main_function();
}

pub fn caller_b() {
    main_function();
    utils::helper();
}

pub struct MyStruct {
    pub field: i32,
}

impl MyStruct {
    pub fn new() -> Self {
        Self { field: utils::helper() }
    }

    pub fn method(&self) -> i32 {
        self.field
    }
}

pub trait MyTrait {
    fn trait_method(&self);
}

impl MyTrait for MyStruct {
    fn trait_method(&self) {
        main_function();
    }
}
