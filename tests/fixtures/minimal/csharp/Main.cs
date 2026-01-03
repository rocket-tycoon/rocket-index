namespace Minimal;

public static class Helpers
{
    public static int Helper()
    {
        return 42;
    }
}

public class Program
{
    public static void MainFunction()
    {
        var x = Helpers.Helper();
        Console.WriteLine(x);
    }

    public static void CallerA()
    {
        MainFunction();
    }

    public static void CallerB()
    {
        MainFunction();
        Helpers.Helper();
    }
}

public class MyClass
{
    private int _field;

    public MyClass()
    {
        _field = Helpers.Helper();
    }

    public virtual int Method()
    {
        return _field;
    }
}

public class ChildClass : MyClass
{
    public override int Method()
    {
        Program.MainFunction();
        return base.Method();
    }
}
