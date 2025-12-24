<?php

require_once 'User.php';
require_once 'PaymentService.php';

$user = new User('Alice Cooper', 'alice@example.com');
echo $user->fullInfo() . "\n";

$service = new PaymentService();
$result = $service->processPayment($user, 150.0);
echo "Payment result: " . ($result ? 'true' : 'false') . "\n";
