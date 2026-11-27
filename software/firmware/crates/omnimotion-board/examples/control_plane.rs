//! Submits a bounded command on core 0 while core 1 exclusively owns `Board`.

#![no_std]
#![no_main]

use embassy_futures::select::{select, Either};
use esp_backtrace as _;
use esp_hal::delay::Delay;
use esp_println::println;
use omnimotion::{
    CommandCompletion, CommandError, Lifecycle, MachineCommand, StandardCommand, StatusSnapshot,
};
use omnimotion_board::control::{snapshot, BoardControlPlane, BoardMachineControl};
use omnimotion_board::{Board, BoardPins, Config};
use static_cell::StaticCell;

esp_bootloader_esp_idf::esp_app_desc!();

const DEPTH: usize = 4;

#[esp_hal::main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let (rtos, pins) = omnimotion_board::split!(peripherals);
    static PLANE: StaticCell<BoardControlPlane<(), DEPTH>> = StaticCell::new();
    let plane = PLANE.init(BoardControlPlane::new(StatusSnapshot::default()));
    let (client, machine) = plane.split();

    omnimotion_board::boot_on_second_core(rtos, move |spawner| {
        spawner.spawn(board_task(pins, machine).expect("board task spawned once"))
    });

    let delay = Delay::new();
    while client.status().available_axes == 0 {
        delay.delay_millis(10);
    }

    for command in [
        MachineCommand::Standard(StandardCommand::Pause),
        MachineCommand::Standard(StandardCommand::Resume),
        MachineCommand::Standard(StandardCommand::Stop),
        MachineCommand::Application(()),
    ] {
        let id = client.submit(command).expect("command queue has room");
        loop {
            if let Some(completion) = client.take_completion(id) {
                println!("request {:?}, completion {:?}", id, completion);
                break;
            }
            delay.delay_millis(1);
        }
    }

    loop {
        delay.delay_millis(5_000);
        println!("status: {:?}", client.status());
    }
}

#[embassy_executor::task]
async fn board_task(pins: BoardPins, machine: BoardMachineControl<'static, (), DEPTH>) {
    let mut board = Board::start(pins, Config::default())
        .await
        .expect("board startup");
    let mut lifecycle = Lifecycle::new();
    lifecycle.start(u32::MAX).expect("valid example job");
    lifecycle.begin_operation().expect("first operation");
    machine.publish(snapshot(
        &board,
        lifecycle.state(),
        None,
        lifecycle.progress(),
    ));

    loop {
        // Enabled outputs arm the watchdog, which only the supervisor feeds.
        let request = if board.faults.drivers_enabled() {
            match select(machine.next(), board.faults.wait_for_fault()).await {
                Either::First(request) => request,
                Either::Second(fault) => {
                    println!("fault: {fault:?}");
                    lifecycle.fault();
                    machine.publish(snapshot(
                        &board,
                        lifecycle.state(),
                        None,
                        lifecycle.progress(),
                    ));
                    continue;
                }
            }
        } else {
            machine.next().await
        };
        machine.publish(snapshot(
            &board,
            lifecycle.state(),
            Some(request.id),
            lifecycle.progress(),
        ));
        let result = match request.command {
            MachineCommand::Standard(StandardCommand::Pause) => match lifecycle.request_pause() {
                Ok(needs_abort) => {
                    if needs_abort {
                        let abort = board.motion.abort_handle();
                        let ticket = abort.request();
                        abort.wait(ticket).await;
                        lifecycle
                            .acknowledge_interruption()
                            .expect("abort acknowledged");
                        lifecycle
                            .declare_safe_boundary()
                            .expect("application recovery boundary");
                    }
                    lifecycle.pause().map_err(|_| CommandError::InvalidState)
                }
                Err(_) => Err(CommandError::InvalidState),
            },
            MachineCommand::Standard(StandardCommand::Resume) => lifecycle
                .resume()
                .and_then(|_| lifecycle.begin_operation().map(|_| ()))
                .map_err(|_| CommandError::InvalidState),
            MachineCommand::Standard(StandardCommand::Stop) => match lifecycle.request_stop() {
                Ok(needs_abort) => {
                    if needs_abort {
                        let abort = board.motion.abort_handle();
                        let ticket = abort.request();
                        abort.wait(ticket).await;
                        lifecycle
                            .acknowledge_interruption()
                            .expect("abort acknowledged");
                    }
                    lifecycle
                        .acknowledge_stop()
                        .map_err(|_| CommandError::InvalidState)
                }
                Err(_) => Err(CommandError::InvalidState),
            },
            MachineCommand::Standard(StandardCommand::EnableDrivers) => board
                .enable()
                .await
                .map(|_| ())
                .map_err(|_| CommandError::InvalidState),
            MachineCommand::Standard(StandardCommand::DisableDrivers) => {
                board.faults.disable_drivers();
                Ok(())
            }
            MachineCommand::Standard(StandardCommand::ClearFault) => {
                if board.faults.clear(&mut board.motion) {
                    let _ = lifecycle.recover_fault();
                    Ok(())
                } else {
                    Err(CommandError::InvalidState)
                }
            }
            // Homing needs application-owned sensor configuration.
            MachineCommand::Standard(StandardCommand::Home { .. }) => {
                Err(CommandError::InvalidCommand)
            }
            MachineCommand::Application(()) => {
                lifecycle.fault();
                Err(CommandError::FaultLatched)
            }
        };
        machine.complete(CommandCompletion {
            id: request.id,
            result,
            reason: None,
        });
        machine.publish(snapshot(
            &board,
            lifecycle.state(),
            None,
            lifecycle.progress(),
        ));
    }
}
