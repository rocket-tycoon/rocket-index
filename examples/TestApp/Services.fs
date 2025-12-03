/// Application services that orchestrate domain operations
module TestApp.Services

open System
open TestApp.Domain

// ----------------------------------------------------------------------------
// Task Repository (in-memory for testing)

type TaskRepository() =
    let mutable tasks: Map<Guid, Task> = Map.empty

    member _.Add(task: Task) =
        tasks <- Map.add task.Id task tasks
        Success task

    member _.GetById(id: Guid) =
        match Map.tryFind id tasks with
        | Some task -> Success task
        | None -> NotFound $"Task with ID {id} not found"

    member _.Update(task: Task) =
        if Map.containsKey task.Id tasks then
            tasks <- Map.add task.Id task tasks
            Success task
        else
            NotFound $"Task with ID {task.Id} not found"

    member _.Delete(id: Guid) =
        if Map.containsKey id tasks then
            tasks <- Map.remove id tasks
            Success ()
        else
            NotFound $"Task with ID {id} not found"

    member _.GetAll() =
        tasks |> Map.values |> Seq.toList

    member _.GetByStatus(status: TaskStatus) =
        tasks
        |> Map.values
        |> Seq.filter (fun t -> t.Status = status)
        |> Seq.toList

    member _.GetOverdue() =
        tasks
        |> Map.values
        |> Seq.filter isOverdue
        |> Seq.toList

// ----------------------------------------------------------------------------
// Task Service

type TaskService(repository: TaskRepository) =

    /// Create a new task
    member _.CreateTask(title: string, priority: Priority) =
        if String.IsNullOrWhiteSpace(title) then
            ValidationError "Title cannot be empty"
        else
            let task = createTask title priority
            repository.Add(task)

    /// Get a task by ID
    member _.GetTask(id: Guid) =
        repository.GetById(id)

    /// Start working on a task
    member _.StartTask(id: Guid) =
        match repository.GetById(id) with
        | Success task ->
            match task.Status with
            | Pending ->
                let updated = updateStatus InProgress task
                repository.Update(updated)
            | status ->
                ValidationError $"Cannot start task with status {status}"
        | error -> error

    /// Complete a task
    member _.CompleteTask(id: Guid) =
        match repository.GetById(id) with
        | Success task ->
            match task.Status with
            | Pending | InProgress ->
                let updated = updateStatus Completed task
                repository.Update(updated)
            | status ->
                ValidationError $"Cannot complete task with status {status}"
        | error -> error

    /// Assign a task to a user
    member _.AssignTask(taskId: Guid, userId: UserId) =
        match repository.GetById(taskId) with
        | Success task ->
            let updated = assignTo userId task
            repository.Update(updated)
        | error -> error

    /// Set due date for a task
    member _.SetDueDate(taskId: Guid, dueDate: DateTime) =
        match repository.GetById(taskId) with
        | Success task ->
            if dueDate < DateTime.UtcNow then
                ValidationError "Due date cannot be in the past"
            else
                let updated = setDueDate dueDate task
                repository.Update(updated)
        | error -> error

    /// Get all tasks sorted by priority
    member _.GetAllTasksSortedByPriority() =
        repository.GetAll()
        |> List.sortWith compareByPriority

    /// Get all overdue tasks
    member _.GetOverdueTasks() =
        repository.GetOverdue()

    /// Get tasks by status
    member _.GetTasksByStatus(status: TaskStatus) =
        repository.GetByStatus(status)
