import { User } from './user';

export class PaymentService {
    processPayment(user: User, amount: number): { success: boolean; amount: number; user: string } {
        console.log(`Processing $${amount} for ${user.name}`);
        return { success: true, amount, user: user.name };
    }

    refundPayment(user: User, amount: number): { success: boolean; amount: number } {
        console.log(`Refunding $${amount} to ${user.name}`);
        return { success: true, amount };
    }
}
