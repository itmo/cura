# A sort of an Arc-RwLock combination.. 

That aims to be simple and straightforward to use when sharing data
between threads. 

An attempt at creating an Arc-RwLock combination that is straightforward
to use and no hassle , instead of worrying about being fast and lean. 

 * cloning referefences works like Arc
 * made for sharing objects between threads without worry
 * locking things works like RwLock with write() or read()
 * it spins a few times and then queues if a lock is not obtained
 * miri seems to be happy , so i trust it doesnt leak too much memory etc.
 * requires that everything you stick into it is Send+Sync
 * no need to constantly .unwrap() things instead it will just
   block forever or blow up

# Example
```rust
use cura::Cura;
let t:i32=1;
let foo=Cura::new(t);
let a=foo.clone();
let b=foo.clone();

{
    assert_eq!(*a.read(),1);
    {
        a.alter(|s|{
            Some(2)
        });
    }
    let lock=a.read();
    let v=*lock;
    assert_eq!(v,2)
}//lock dropped here
{
    (*b.write())+=1; //lock dropped here i think 
}

assert_eq!((*a.read()),3);
```

