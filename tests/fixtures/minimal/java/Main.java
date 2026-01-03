package minimal;

public class Main {
    public static int helper() {
        return 42;
    }

    public static void mainFunction() {
        int x = helper();
        System.out.println(x);
    }

    public static void callerA() {
        mainFunction();
    }

    public static void callerB() {
        mainFunction();
        helper();
    }
}

class MyClass {
    private int field;

    public MyClass() {
        this.field = Main.helper();
    }

    public int method() {
        return this.field;
    }
}

class ChildClass extends MyClass {
    @Override
    public int method() {
        Main.mainFunction();
        return super.method();
    }
}
