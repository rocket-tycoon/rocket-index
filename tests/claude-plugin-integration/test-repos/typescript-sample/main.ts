import { User } from './user';
import { PaymentService } from './payment_service';

function main() {
    const user = new User('Alice Cooper', 'alice@example.com');
    console.log(user.fullInfo());

    const service = new PaymentService();
    const result = service.processPayment(user, 150);
    console.log(`Payment result: ${result.success}`);
}

main();
