const { createUser } = require('./user');
const { PaymentService } = require('./payment_service');

function main() {
    const user = createUser('Alice Cooper', 'alice@example.com');
    console.log(user.fullInfo());

    const service = new PaymentService();
    const result = service.processPayment(user, 150);
    console.log(`Payment result: ${result.success}`);
}

main();
