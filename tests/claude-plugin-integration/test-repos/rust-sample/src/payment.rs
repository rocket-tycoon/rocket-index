use crate::user::User;

pub struct PaymentService;

impl PaymentService {
    pub fn new() -> Self {
        PaymentService
    }

    pub fn process_payment(&self, user: &User, amount: f64) -> bool {
        println!("Processing ${} for {}", amount, user.name);
        true
    }

    pub fn refund_payment(&self, user: &User, amount: f64) -> bool {
        println!("Refunding ${} to {}", amount, user.name);
        true
    }
}
