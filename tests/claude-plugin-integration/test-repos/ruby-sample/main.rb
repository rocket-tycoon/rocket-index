require_relative 'user'
require_relative 'payment_service'

def main
  user = User.new('Alice Cooper', 'alice@example.com')
  puts user.full_info

  service = PaymentService.new
  result = service.process_payment(user, 150)
  puts "Payment result: #{result[:success]}"
end

main if __FILE__ == $PROGRAM_NAME
