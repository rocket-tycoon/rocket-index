class User
  attr_reader :name, :email

  def initialize(name, email)
    @name = name
    @email = email
  end

  def full_info
    "#{@name} <#{@email}>"
  end
end
