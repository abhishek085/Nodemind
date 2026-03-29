import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Task, Suggestion } from "../types";
import SuggestionModal from "../components/SuggestionModal";

export default function TaskListView() {
  const [tasks, setTasks] = useState<Task[]>([]);
  const [newTitle, setNewTitle] = useState("");
  const [newProject, setNewProject] = useState("");
  const [newDue, setNewDue] = useState("");
  const [calendarModalOpen, setCalendarModalOpen] = useState(false);
  const [selectedTaskForCalendar, setSelectedTaskForCalendar] = useState<Task | null>(null);

  const loadTasks = async () => {
    try {
      const result: Task[] = await invoke("get_tasks");
      setTasks(result);
    } catch {
      setTasks([]);
    }
  };

  useEffect(() => {
    loadTasks();
    const id = setInterval(loadTasks, 5000);
    return () => clearInterval(id);
  }, []);

  const activeTasks = useMemo(
    () => tasks.filter((task) => !task.done),
    [tasks],
  );

  const activeByProject = useMemo(() => {
    const groups: Record<string, Task[]> = {};
    for (const task of activeTasks) {
      const key = task.project?.trim() || "Inbox";
      if (!groups[key]) groups[key] = [];
      groups[key].push(task);
    }
    return groups;
  }, [activeTasks]);

  const completedCount = tasks.length - activeTasks.length;

  const addTask = async () => {
    const title = newTitle.trim();
    if (!title) return;

    await invoke("create_task", {
      title,
      project: newProject.trim() || null,
      dueHint: newDue.trim() || null,
    });

    setNewTitle("");
    setNewProject("");
    setNewDue("");
    await loadTasks();
  };

  const completeTask = async (id: string) => {
    await invoke("mark_task_done", { id });
    await loadTasks();
  };

  const openCalendarModal = (task: Task) => {
    setSelectedTaskForCalendar(task);
    setCalendarModalOpen(true);
  };

  const handleCalendarSave = async () => {
    setCalendarModalOpen(false);
    setSelectedTaskForCalendar(null);
  };

  return (
    <div className="view-today">
      <div className="view-header">
        <div>
          <h1 className="view-title">Task List</h1>
          <p className="view-subtitle">All created tasks appear here until you mark them completed.</p>
        </div>
      </div>

      <div className="card status-card" style={{ marginBottom: "1rem" }}>
        <div className="quick-add">
          <input
            className="qa-input"
            placeholder="New task..."
            value={newTitle}
            onChange={(e) => setNewTitle(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && addTask()}
          />
          <input
            className="qa-input qa-small"
            placeholder="Project"
            value={newProject}
            onChange={(e) => setNewProject(e.target.value)}
          />
          <input
            className="qa-input qa-small"
            placeholder="Due"
            value={newDue}
            onChange={(e) => setNewDue(e.target.value)}
          />
          <button className="qa-btn" onClick={addTask}>
            +
          </button>
        </div>
      </div>

      {activeTasks.length === 0 ? (
        <div className="card empty-state">No open tasks. Accepted suggestions and manual tasks will show here.</div>
      ) : (
        <div className="card" style={{ padding: "1rem" }}>
          {Object.entries(activeByProject).map(([project, projectTasks]) => (
            <div key={project} className="task-group">
              <div className="task-group-label">{project}</div>
              {projectTasks.map((task) => (
                <div key={task.id} className="task-row">
                  <button
                    className="task-check"
                    onClick={() => completeTask(task.id)}
                    title="Mark done"
                  >
                    ○
                  </button>
                  <div className="task-body">
                    <span className="task-title">{task.title}</span>
                    {task.due_hint && (
                      <span className="task-due">{task.due_hint}</span>
                    )}
                  </div>
                  <button
                    className="task-schedule-btn"
                    onClick={() => openCalendarModal(task)}
                    title="Schedule to calendar"
                  >
                    📅
                  </button>
                  <span className={`task-source src-${task.source}`}>{task.source}</span>
                </div>
              ))}
            </div>
          ))}
        </div>
      )}

      {completedCount > 0 && (
        <div className="card" style={{ marginTop: "1rem", padding: "0.8rem 1rem", opacity: 0.8 }}>
          {completedCount} completed task{completedCount === 1 ? "" : "s"} hidden from this list.
        </div>
      )}

      {selectedTaskForCalendar && (
        <SuggestionModal
          suggestion={{
            text: selectedTaskForCalendar.title,
            type: "task",
            priority: "medium",
          } as Suggestion}
          isOpen={calendarModalOpen}
          onClose={() => setCalendarModalOpen(false)}
          onSave={handleCalendarSave}
        />
      )}
    </div>
  );
}
