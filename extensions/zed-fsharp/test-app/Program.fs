/// Main entry point - demonstrates usage of Domain and Services
module TestApp.Program

open System
open TestApp.Domain
open TestApp.Services

/// Print a task to console
let printTask (task: Task) =
    let assignee =
        match task.AssignedTo with
        | Some (UserId id) -> $"Assigned to: {id}"
        | None -> "Unassigned"

    let dueDate =
        match task.DueDate with
        | Some date -> sprintf "Due: %s" (date.ToString("yyyy-MM-dd"))
        | None -> "No due date"

    let overdue = if isOverdue task then " [OVERDUE]" else ""

    printfn "  [%A] %s - %A%s" task.Priority task.Title task.Status overdue
    printfn "       %s | %s" assignee dueDate

/// Print a list of tasks
let printTasks title tasks =
    printfn "\n=== %s ===" title
    match tasks with
    | [] -> printfn "  (no tasks)"
    | _ -> tasks |> List.iter printTask

/// Handle a domain result
let handleResult description = function
    | Success value ->
        printfn "SUCCESS: %s" description
        Some value
    | ValidationError msg ->
        printfn "VALIDATION ERROR: %s" msg
        None
    | NotFound msg ->
        printfn "NOT FOUND: %s" msg
        None

[<EntryPoint>]
let main args =
    printfn "F# Task Manager - Test Application"
    printfn "=================================="

    // Create services
    let repository = TaskRepository()
    let taskService = TaskService(repository)

    // Create a user
    let userId = newUserId()
    printfn "\nCreated user: %A" (unwrapUserId userId)

    // Create some tasks
    printfn "\n--- Creating Tasks ---"

    let task1 =
        taskService.CreateTask("Implement login feature", High)
        |> handleResult "Created high priority task"

    let task2 =
        taskService.CreateTask("Write documentation", Low)
        |> handleResult "Created low priority task"

    let task3 =
        taskService.CreateTask("Fix critical bug", Critical)
        |> handleResult "Created critical task"

    let task4 =
        taskService.CreateTask("Review pull request", Medium)
        |> handleResult "Created medium priority task"

    // Try to create invalid task
    taskService.CreateTask("", Medium)
    |> handleResult "Tried to create empty task"
    |> ignore

    // Show all tasks sorted by priority
    printTasks "All Tasks (by priority)" (taskService.GetAllTasksSortedByPriority())

    // Start and complete some tasks
    printfn "\n--- Updating Tasks ---"

    match task1 with
    | Some t ->
        taskService.StartTask(t.Id)
        |> handleResult "Started task 1"
        |> ignore

        taskService.AssignTask(t.Id, userId)
        |> handleResult "Assigned task 1"
        |> ignore
    | None -> ()

    match task3 with
    | Some t ->
        taskService.StartTask(t.Id)
        |> handleResult "Started task 3"
        |> ignore

        taskService.CompleteTask(t.Id)
        |> handleResult "Completed task 3"
        |> ignore
    | None -> ()

    // Set a due date in the past to test overdue detection
    match task2 with
    | Some t ->
        // This should fail - due date in the past
        taskService.SetDueDate(t.Id, DateTime.UtcNow.AddDays(-1.0))
        |> handleResult "Tried to set past due date"
        |> ignore

        // This should succeed
        taskService.SetDueDate(t.Id, DateTime.UtcNow.AddDays(7.0))
        |> handleResult "Set future due date"
        |> ignore
    | None -> ()

    // Show tasks by status
    printTasks "Pending Tasks" (taskService.GetTasksByStatus(Pending))
    printTasks "In Progress Tasks" (taskService.GetTasksByStatus(InProgress))
    printTasks "Completed Tasks" (taskService.GetTasksByStatus(Completed))

    // Show overdue tasks
    printTasks "Overdue Tasks" (taskService.GetOverdueTasks())

    // Final state
    printTasks "Final State (all tasks)" (taskService.GetAllTasksSortedByPriority())

    printfn "\n--- Done ---"
    0
