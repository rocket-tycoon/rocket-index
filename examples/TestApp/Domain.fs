/// Domain types and core business logic
module TestApp.Domain

open System

// ----------------------------------------------------------------------------
// Types

/// Represents a unique identifier
type UserId = UserId of Guid

/// Status of a task
type TaskStatus =
    | Pending
    | InProgress
    | Completed
    | Cancelled

/// Priority level for tasks
type Priority =
    | Low
    | Medium
    | High
    | Critical

/// A task record
type Task = {
    Id: Guid
    Title: string
    Description: string option
    Status: TaskStatus
    Priority: Priority
    AssignedTo: UserId option
    CreatedAt: DateTime
    DueDate: DateTime option
}

/// Result of a domain operation
type DomainResult<'T> =
    | Success of 'T
    | ValidationError of string
    | NotFound of string

// ----------------------------------------------------------------------------
// Functions

/// Create a new user ID
let newUserId () = UserId (Guid.NewGuid())

/// Extract the GUID from a UserId
let unwrapUserId (UserId id) = id

/// Create a new task with default values
let createTask title priority =
    {
        Id = Guid.NewGuid()
        Title = title
        Description = None
        Status = Pending
        Priority = priority
        AssignedTo = None
        CreatedAt = DateTime.UtcNow
        DueDate = None
    }

/// Update task status
let updateStatus newStatus task =
    { task with Status = newStatus }

/// Assign task to a user
let assignTo userId task =
    { task with AssignedTo = Some userId }

/// Set the due date for a task
let setDueDate date task =
    { task with DueDate = Some date }

/// Check if a task is overdue
let isOverdue task =
    match task.DueDate with
    | Some dueDate -> DateTime.UtcNow > dueDate && task.Status <> Completed
    | None -> false

/// Get priority weight for sorting
let priorityWeight = function
    | Critical -> 4
    | High -> 3
    | Medium -> 2
    | Low -> 1

/// Compare tasks by priority (higher priority first)
let compareByPriority task1 task2 =
    compare (priorityWeight task2.Priority) (priorityWeight task1.Priority)
