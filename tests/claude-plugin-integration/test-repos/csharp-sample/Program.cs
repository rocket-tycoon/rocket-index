using System;

class Program
{
    static void Main(string[] args)
    {
        var user = new User("Alice Cooper", "alice@example.com");
        Console.WriteLine(user.FullInfo());

        var service = new PaymentService();
        var result = service.ProcessPayment(user, 150.0);
        Console.WriteLine($"Payment result: {result}");
    }
}
