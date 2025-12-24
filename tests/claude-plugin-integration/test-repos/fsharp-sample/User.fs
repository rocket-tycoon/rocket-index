module User =
    type User = {
        Name: string
        Email: string
    }

    let create name email =
        { Name = name; Email = email }

    let fullInfo user =
        sprintf "%s <%s>" user.Name user.Email
