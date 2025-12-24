<?php

require_once 'User.php';

class PaymentService {
    public function processPayment(User $user, float $amount): bool {
        echo "Processing \${$amount} for {$user->name}\n";
        return true;
    }

    public function refundPayment(User $user, float $amount): bool {
        echo "Refunding \${$amount} to {$user->name}\n";
        return true;
    }
}
