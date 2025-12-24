class PaymentService {
    processPayment(user, amount) {
        console.log(`Processing $${amount} for ${user.name}`);
        return { success: true, amount, user: user.name };
    }

    refundPayment(user, amount) {
        console.log(`Refunding $${amount} to ${user.name}`);
        return { success: true, amount };
    }
}

module.exports = { PaymentService };
