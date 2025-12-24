public class PaymentService {
    public boolean processPayment(User user, double amount) {
        System.out.printf("Processing $%.2f for %s%n", amount, user.getName());
        return true;
    }

    public boolean refundPayment(User user, double amount) {
        System.out.printf("Refunding $%.2f to %s%n", amount, user.getName());
        return true;
    }
}
