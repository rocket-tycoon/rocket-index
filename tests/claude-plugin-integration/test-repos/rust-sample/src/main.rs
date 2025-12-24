mod user;
mod payment;

use user::User;
use payment::PaymentService;

fn main() {
    let user = User::new("Alice Cooper".to_string(), "alice@example.com".to_string());
    println!("{}", user.full_info());

    let service = PaymentService::new();
    let result = service.process_payment(&user, 150.0);
    println!("Payment result: {}", result);
}
