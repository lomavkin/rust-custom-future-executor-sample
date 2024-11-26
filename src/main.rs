use futures::{
    future::join,
    future::Future,
    future::{BoxFuture, FutureExt},
    task::{waker_ref, ArcWake},
};
use std::time::Duration;
use std::{
    pin::Pin,
    sync::mpsc::{sync_channel, Receiver, SyncSender},
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
    thread,
};

// TimerFuture is a simple future that will complete after a duration has elapsed
pub struct TimerFuture {
    shared_state: Arc<Mutex<SharedState>>,
}

// SharedState has a completed field to indicate whether the timer has completed or not
// and a waker field to store the waker that is used to wake up the TimerFuture when the timer completes
struct SharedState {
    completed: bool,      // Whether the timer has completed or not
    waker: Option<Waker>, // Waker to wake up the timer when it completes
}

// Implement Future for TimerFuture
impl Future for TimerFuture {
    type Output = ();
    // The poll function is called when the future is being polled
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Lock the shared state
        let mut shared_state = self.shared_state.lock().unwrap();
        if shared_state.completed {
            // If the timer has completed, return Poll::Ready
            Poll::Ready(())
        } else {
            // If the timer has not completed, store the waker
            shared_state.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

// Implement TimerFuture
impl TimerFuture {
    // Create a new TimerFuture with the given duration
    pub fn new(duration: Duration) -> Self {
        let shared_state = Arc::new(Mutex::new(SharedState {
            completed: false,
            waker: None,
        }));

        let thread_shared_state = shared_state.clone();
        thread::spawn(move || {
            thread::sleep(duration);
            let mut shared_state = thread_shared_state.lock().unwrap();
            // Future is now completed
            shared_state.completed = true;
            if let Some(waker) = shared_state.waker.take() {
                // Wake up the waker
                waker.wake()
            }
        });

        TimerFuture { shared_state }
    }
}

// Define an Executor struct that holds a receiver of tasks
struct Executor {
    ready_queue: Receiver<Arc<Task>>, // Receiver of tasks
}

impl Executor {
    fn run(&self) {
        while let Ok(task) = self.ready_queue.recv() {
            let mut future_slot = task.future.lock().unwrap();
            if let Some(mut future) = future_slot.take() {
                // get waker from task. waker_ref called Task::wake_by_ref
                let waker = waker_ref(&task);
                // get context from waker
                let context = &mut Context::from_waker(&waker);
                if future.as_mut().poll(context).is_pending() {
                    // If the future has not yet completed, put it back in the task
                    *future_slot = Some(future);
                }
            }
        }
    }
}

// Define a Spawner struct that holds a sender of tasks
#[derive(Clone)]
struct Spawner {
    task_sender: SyncSender<Arc<Task>>, // Sender of tasks
}

impl Spawner {
    // Spawn a future onto the executor
    fn spawn(&self, future: impl Future<Output = ()> + 'static + Send) {
        let future = future.boxed();
        let task = Arc::new(Task {
            future: Mutex::new(Some(future)),
            task_sender: self.task_sender.clone(),
        });
        // send task to executor
        self.task_sender.send(task).expect("too many tasks queued");
    }
}

// Define a Task struct that holds a future and a sender of tasks
struct Task {
    future: Mutex<Option<BoxFuture<'static, ()>>>, // Future to poll
    task_sender: SyncSender<Arc<Task>>,            // Sender of tasks. for wake_by_ref
}

// Implement ArcWake for Task
impl ArcWake for Task {
    // wake is called when the future should be awoken
    fn wake_by_ref(arc_self: &Arc<Self>) {
        let cloned = arc_self.clone();
        arc_self
            .task_sender
            .send(cloned) // send task to executor again
            .expect("too many tasks queued");
    }
}

// Create a new executor and spawner
fn new_executor_and_spawner() -> (Executor, Spawner) {
    const MAX_QUEUED_TASKS: usize = 10_000;
    let (task_sender, ready_queue) = sync_channel(MAX_QUEUED_TASKS);
    (Executor { ready_queue }, Spawner { task_sender })
}

async fn learn_song() -> String {
    println!("Learning the song...");
    TimerFuture::new(Duration::new(2, 0)).await;
    println!("Learned the song!");
    "La La La".to_string()
}

async fn sing_song(song: String) {
    println!("Singing the song: {}", song);
    TimerFuture::new(Duration::new(2, 0)).await;
    println!("Finished singing the song!");
}

async fn learn_and_sing() {
    let song = learn_song().await;
    sing_song(song).await;
}

async fn dance() {
    for _ in 0..3 {
        println!("Dancing...");
        TimerFuture::new(Duration::new(1, 0)).await;
    }
    println!("Finished dancing!");
}

async fn run_tasks() {
    let f1 = learn_and_sing();
    let f2 = dance();
    join(f1, f2).await;
}

fn main() {
    // Create a new executor and spawner
    let (executor, spawner) = new_executor_and_spawner();

    // Spawn a task to learn a song and sing it
    spawner.spawn(async {
        println!("---- start tasks ----");
        run_tasks().await;
        println!("---- finish tasks ----");
    });

    // Drop the spawner, so that the executor knows it should exit once all tasks are done
    drop(spawner);

    executor.run();
}
