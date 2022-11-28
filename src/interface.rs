use ferrite::{
    variable::{sync::ValueGuard, *},
    Context,
};
use std::{fmt::Display, ops::Deref};

pub fn take_var<V: Var>(ctx: &mut Context, prefix: &str, name: &str) -> V
where
    AnyVariable: Downcast<V>,
{
    log::trace!("Interface: {}:{}", prefix, name);
    let any = ctx
        .registry
        .remove(name)
        .unwrap_or_else(|| panic!("No such name: {}", name));
    let info = any.info();
    any.downcast()
        .unwrap_or_else(|| panic!("Bad type, {:?} expected", info))
}

pub struct Interface {
    pub ser_numb: IfaceVariable<String, ArrayVariable<u8>, StringAdapter>,
    pub out_ena: IfaceVariable<u16, Variable<u16>, ScalarAdapter>,
    pub volt_real: IfaceVariable<f64, Variable<f64>, ScalarAdapter>,
    pub curr_real: IfaceVariable<f64, Variable<f64>, ScalarAdapter>,
    pub over_volt_set_point: IfaceVariable<f64, Variable<f64>, ScalarAdapter>,
    pub under_volt_set_point: IfaceVariable<f64, Variable<f64>, ScalarAdapter>,
    pub volt_set: IfaceVariable<f64, Variable<f64>, ScalarAdapter>,
    pub curr_set: IfaceVariable<f64, Variable<f64>, ScalarAdapter>,
}

impl Interface {
    pub fn new(ctx: &mut Context, prefix: &str) -> Self {
        Self {
            ser_numb: IfaceVariable::new(take_var(ctx, prefix, "ser_numb")),
            out_ena: IfaceVariable::new(take_var(ctx, prefix, "out_ena")),
            volt_real: IfaceVariable::new(take_var(ctx, prefix, "volt_real")),
            curr_real: IfaceVariable::new(take_var(ctx, prefix, "curr_real")),
            over_volt_set_point: IfaceVariable::new(take_var(ctx, prefix, "over_volt_set_point")),
            under_volt_set_point: IfaceVariable::new(take_var(ctx, prefix, "under_volt_set_point")),
            volt_set: IfaceVariable::new(take_var(ctx, prefix, "volt_set")),
            curr_set: IfaceVariable::new(take_var(ctx, prefix, "curr_set")),
        }
    }
}

pub trait Adapter<T, V: VarSync>: Default {
    fn load(&self, guard: &ValueGuard<V>) -> T;
    fn store(&self, guard: &mut ValueGuard<V>, value: &T);
}

#[derive(Clone, Default, Debug)]
pub struct ScalarAdapter;

impl<T: Copy> Adapter<T, Variable<T>> for ScalarAdapter {
    fn load(&self, guard: &ValueGuard<Variable<T>>) -> T {
        **guard
    }
    fn store(&self, guard: &mut ValueGuard<Variable<T>>, value: &T) {
        **guard = *value;
    }
}

#[derive(Clone, Default, Debug)]
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
    pub fn new(variable: V) -> Self {
        Self {
            variable,
            adapter: A::default(),
            last_value: None,
        }
    }

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
    pub async fn write<R: Display>(&mut self, result: Result<T, R>) {
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
    pub async fn reject<R: Display>(self, reason: R) {
        self.guard.reject(&format!("{}", reason)).await;
    }
}

impl<'a, T, V: VarSync, A: Adapter<T, V>> Deref for ReadGuard<'a, T, V, A> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.value
    }
}
