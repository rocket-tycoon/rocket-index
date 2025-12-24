from user import User

class PaymentService:
    def process_payment(self, user: User, amount: float) -> dict:
        print(f'Processing ${amount} for {user.name}')
        return {'success': True, 'amount': amount, 'user': user.name}

    def refund_payment(self, user: User, amount: float) -> dict:
        print(f'Refunding ${amount} to {user.name}')
        return {'success': True, 'amount': amount}
