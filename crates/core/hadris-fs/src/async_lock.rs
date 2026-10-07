pub(crate) type Guard<'a, T> = async_lock::MutexGuard<'a, T>;

pub(crate) struct Lock<T>(async_lock::Mutex<T>);

impl<T> Lock<T> {
    pub(crate) fn new(value: T) -> Self {
        Self(async_lock::Mutex::new(value))
    }

    pub(crate) async fn lock(&self) -> Guard<'_, T> {
        self.0.lock().await
    }

    pub(crate) fn try_lock(&self) -> Option<Guard<'_, T>> {
        self.0.try_lock()
    }

    pub(crate) fn into_inner(self) -> T {
        self.0.into_inner()
    }
}
