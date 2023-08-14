use futures::prelude::*;

use sp_consensus::Error as ConsensusError;
use tokio::time::{sleep, Duration};


struct HotstuffWork {
}

impl HotstuffWork {
    async fn do_work(&self) {
        // Implement your actual work logic here
        println!("Performing HotstuffWork...");
    }
}

// pub fn start_hotstuff()  -> Result<impl Future<Output = ()>, ConsensusError> {
//     println!("Hello hotstuff");

//     let work = HotstuffWork {};

//     let futureWork = async move {
//         work.do_work().await;
//         // Perform your async work here using HotstuffWork
//         println!("👷🏻‍♂️HotstuffWork completed.");
//     };


//     Ok(start_hotstuff_task())
// }

pub async fn start_hotstuff_task() {

	loop {
        // For example, you can await some asynchronous operations.
        sleep(Duration::from_millis(3000)).await;
   
        println!("3000 ms have elapsed");
	}
}

pub fn start_hotstuff() -> Result<impl Future<Output = ()>, ConsensusError> {
    println!("Hello hotstuff");

    let work = HotstuffWork {};

    let future_work = async move {
        loop {
            work.do_work().await;
            sleep(Duration::from_millis(3000)).await;
            println!("3000 ms have elapsed");
        }
    };

    Ok(future_work)
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_start_hotstuff() {
        // Arrange
        let result = start_hotstuff().expect("hotstuff entry function");

        // Assert
        // Add your assertions here if needed

    }
}