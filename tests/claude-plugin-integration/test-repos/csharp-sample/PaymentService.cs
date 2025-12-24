using System;

public class PaymentService
{
    public bool ProcessPayment(User user, double amount)
    {
        Console.WriteLine($"Processing ${amount:F2} for {user.Name}");
        return true;
    }

    public bool RefundPayment(User user, double amount)
    {
        Console.WriteLine($"Refunding ${amount:F2} to {user.Name}");
        return true;
    }
}
