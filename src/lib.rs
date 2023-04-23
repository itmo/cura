use std::ops::{Deref,DerefMut};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicUsize,AtomicI32};
use std::sync::atomic::Ordering::{Acquire, Relaxed, Release};
/**
 *  a sort of an Arc that will both readwrite lock , be easy to
 *  handle and is cloneable and assignable.
 * ```
 *  assert_eq!(0,0);
 * ```
 *
 *
 */
pub struct Cura<T: Sync + Send> {
    ptr: NonNull<CuraData<T>>,
}
struct CuraData<T: Sync + Send> {
    count: AtomicUsize,
    data: T,
    lockcount:AtomicI32, //-999=writeĺock,0=free,>0 readlock count
}
impl<T: Sync + Send> Cura<T> {
    pub fn new(t: T) -> Cura<T> {
        Cura {
            ptr: NonNull::from(Box::leak(Box::new(CuraData {
                count: AtomicUsize::new(1),
                data: t,
                lockcount:AtomicI32::new(0),
            }))),
        }
    }
    fn data(&self) -> &CuraData<T> {
        unsafe { self.ptr.as_ref() }
    }
    //std::hint::spin_loop();
    //thread::park()
    pub fn read(&self)->ReadGuard<T>
    {
        //check that there are no writelocks and then 
        //return a new readlock
        //spinlock a few times , then enter queue and park on a condvar
        //fetch_update lockvalue to +1 , unless it is negative in which
        //  case fail 
        todo!("do this stuff");
        ReadGuard{
            cura:self,
        }
    }
    pub fn write(&self)->Guard<T>
    {
        //check that there are no readlocks or writelocks and 
        //return a writelock 
        //implement this by swapping a "0" to "-999" and checking that
        //it indeed was  "0"
        //self.locked.compare_exchange_weak(
          //  false, true, Acquire, Relaxed).is_err()
        //if getting writelock fails, insert into queue
        //spinlock a few times, then go to queue
        todo!("do this stuff");
        Guard{
            cura:self,
        }
    }
    //pub fn alter
}
/**
 *  implement send and sync since thats all we want
 */
unsafe impl<T: Send + Sync> Send for Cura<T> {}
unsafe impl<T: Send + Sync> Sync for Cura<T> {}

/**
 *  deref to make use simpler, this should also transparently
 *  read-lock
 */
 /*TBD
impl<T:Sync+Send> Deref for Cura<T>
{
    type Target = ReadGuard<T>;
    //TBD this should probably return a reference to readguard?
    fn deref(&self) -> &Self::Target {
        todo!("this deref should actually do a 'read()' ");
        //&self.data().data
        &self.read()
    }
}*/
/**
 *  clone to make new references of the object
 */
impl<T: Sync + Send> Clone for Cura<T> {
    fn clone(&self) -> Self {
        self.data().count.fetch_add(1, Relaxed);
        Cura { ptr: self.ptr }
    }
}
/**
 *  drop to clean up references
 */
impl<T: Sync + Send> Drop for Cura<T> {
    fn drop(&mut self) {
        if self.data().count.fetch_sub(1, Release) == 1 {
            unsafe {
                drop(Box::from_raw(self.ptr.as_ptr()));
            }
        }
    }
}
/**********************************************************
 *  guards
 */
/**
 *  writeguard for Cura
 */
pub struct Guard<'a,T:Send+Sync>
{
    cura:&'a Cura<T>,
}
impl<T:Send+Sync> Drop for Guard<'_,T>
{
    fn drop(&mut self) {
        todo!("unlock guard on Cura and signal waiting threads");
/*        if self.data().count.fetch_sub(1, Release) == 1 {
            unsafe {
                drop(Box::from_raw(self.ptr.as_ptr()));
            }
        }*/
    }
}
impl<T: Sync + Send> Deref for Guard<'_,T> {
    type Target = T;
    fn deref(&self) -> &T {
        todo!("this should return a reference to data");
    }
}
impl<T: Sync + Send> DerefMut for Guard<'_,T> {
    fn deref_mut(&mut self) -> &mut T {
        todo!("this should return a mutable reference to data");
    }
}


/**
 *  readguard for Cura
 */
pub struct ReadGuard<'a,T:Send+Sync>
{
    cura:&'a Cura<T>,
}
impl<T:Send+Sync> Drop for ReadGuard<'_,T>
{
    fn drop(&mut self) {
        todo!("unlock readguard on Cura and signal waiting threads");
    /*
        if self.data().count.fetch_sub(1, Release) == 1 {
            unsafe {
                drop(Box::from_raw(self.ptr.as_ptr()));
            }
        }*/
    }
}
impl<T: Sync + Send> Deref for ReadGuard<'_,T> {
    type Target = T;
    fn deref(&self) -> &T {
        todo!("this should return a reference to data");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn basic_usecases() {

        /*  calculator for hits to "testing"*/
        static NUM_HITS: AtomicUsize = AtomicUsize::new(0);

        /*  testing struct to act as the value*/
        struct Foo {}
        impl Foo {
            pub fn new() -> Foo {
                Foo {}
            }
            pub fn testing(&self) {
                println!("works");
                NUM_HITS.fetch_add(1, Relaxed);
            }
        }
        /*  create ref and hit the test method*/
        let x = Cura::new(Foo::new());
        let y = x.clone();
        y.read().testing(); //TBD convert this to work with deref
        assert_eq!(1, NUM_HITS.load(Acquire));
    }
    #[test]
    fn it_works() {
        static DROPS: AtomicUsize = AtomicUsize::new(0);

        struct Dropped;

        impl Drop for Dropped {
            fn drop(&mut self) {
                DROPS.fetch_add(1, Relaxed);
            }
        }

        /*  create two testobjects and keep track of dropping*/
        /*  by including the drop-detector into a tuple*/
        let x = Cura::new(("salve!", Dropped));
        let y = x.clone();

        /*  push to another thread, see it works there*/
        let t = std::thread::spawn(move || {
            assert_eq!(x.read().0, "salve!");//TBD conver to deref
        });

        /*  and still works here*/
        assert_eq!(y.read().0, "salve!"); //TBD convert to deref

        /*  wait for the thread*/
        t.join().unwrap();

        /*  object shouldnt have dropped yet*/
        assert_eq!(DROPS.load(Relaxed), 0);

        /*  and we drop the last reference here , so it should drop*/
        drop(y);

        /*  and check the result.*/
        assert_eq!(DROPS.load(Relaxed), 1);
    }
}
