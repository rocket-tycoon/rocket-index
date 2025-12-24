require_relative 'user'

class PaymentService
  def process_payment(user, amount)
    puts "Processing $#{amount} for #{user.name}"
    { success: true, amount: amount, user: user.name }
  end

  def refund_payment(user, amount)
    puts "Refunding $#{amount} to #{user.name}"
    { success: true, amount: amount }
  end
end
