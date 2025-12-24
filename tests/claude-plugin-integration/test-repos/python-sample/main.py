from user import User
from payment_service import PaymentService

def main():
    user = User('Alice Cooper', 'alice@example.com')
    print(user.full_info())

    service = PaymentService()
    result = service.process_payment(user, 150)
    print(f"Payment result: {result['success']}")

if __name__ == '__main__':
    main()
