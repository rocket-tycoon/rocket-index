public class Main {
    public static void main(String[] args) {
        User user = new User("Alice Cooper", "alice@example.com");
        System.out.println(user.fullInfo());

        PaymentService service = new PaymentService();
        boolean result = service.processPayment(user, 150.0);
        System.out.println("Payment result: " + result);
    }
}
