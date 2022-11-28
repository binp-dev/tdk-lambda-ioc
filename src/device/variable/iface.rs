use super::Error;
use ferrite::variable::{sync::ValueGuard, *};
use std::ops::Deref;

pub trait Adapter<T, V: VarSync> {
    fn load(&self, guard: &ValueGuard<V>) -> T;
    fn store(&self, guard: &mut ValueGuard<V>, value: &T);
}

pub struct ScalarAdapter;

impl<T: Copy> Adapter<T, Variable<T>> for ScalarAdapter {
    fn load(&self, guard: &ValueGuard<Variable<T>>) -> T {
        **guard
    }
    fn store(&self, guard: &mut ValueGuard<Variable<T>>, value: &T) {
        **guard = *value;
    }
}

pub struct StringAdapter;

impl Adapter<String, ArrayVariable<u8>> for StringAdapter {
    fn load(&self, guard: &ValueGuard<ArrayVariable<u8>>) -> String {
        String::from_utf8_lossy(guard.as_slice()).to_string()
    }
    fn store(&self, guard: &mut ValueGuard<ArrayVariable<u8>>, value: &String) {
        guard.clear();
        guard.extend_from_slice(value.as_bytes());
    }
}

/// Interface-side variable.
pub struct IfaceVariable<T, V: VarSync, A: Adapter<T, V>> {
    variable: V,
    adapter: A,
    last_value: Option<T>,
}

impl<T, V: VarSync, A: Adapter<T, V>> IfaceVariable<T, V, A> {
    pub async fn read(&mut self) -> ReadGuard<'_, T, V, A> {
        let mut guard = self.variable.acquire().await;
        let value = self.adapter.load(&guard);
        if let Some(last_value) = self.last_value.as_ref() {
            self.adapter.store(&mut guard, last_value);
        }
        ReadGuard {
            value,
            guard,
            adapter: &self.adapter,
            last_value: &mut self.last_value,
        }
    }
    pub async fn write(&mut self, result: Result<T, Error>) {
        let mut guard = self.variable.request().await;
        match result {
            Ok(value) => {
                self.adapter.store(&mut guard, &value);
                self.last_value.replace(value);
                guard.accept()
            }
            Err(reason) => guard.reject(&format!("{}", reason)),
        }
        .await;
    }
}

pub struct ReadGuard<'a, T, V: VarSync, A: Adapter<T, V>> {
    value: T,
    guard: ValueGuard<'a, V>,
    adapter: &'a A,
    last_value: &'a mut Option<T>,
}

impl<'a, T, V: VarSync, A: Adapter<T, V>> ReadGuard<'a, T, V, A> {
    pub async fn accept(mut self) {
        self.adapter.store(&mut self.guard, &self.value);
        self.last_value.replace(self.value);
        self.guard.accept().await;
    }
    pub async fn reject(self, reason: Error) {
        self.guard.reject(&format!("{}", reason)).await;
    }
}

impl<'a, T, V: VarSync, A: Adapter<T, V>> Deref for ReadGuard<'a, T, V, A> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.value
    }
}
