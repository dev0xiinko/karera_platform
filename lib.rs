#![cfg_attr(not(feature = "std"), no_std, no_main)]

#[ink::contract]
mod karera_platform {
    use ink::storage::Mapping;
    use ink::prelude::vec::Vec;

    // Race duration: 10 minutes = 600 seconds / 6 seconds per block = 100 blocks
    const RACE_DURATION_BLOCKS: u32 = 100;
    const HORSES_PER_RACE: u8 = 6;
    const TOTAL_RACES: u8 = 5;
    const FINISH_LINE: u32 = 1000; // Distance units to finish

    #[derive(Debug, PartialEq, Eq, Clone, scale::Encode, scale::Decode)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo, ink::storage::traits::StorageLayout))]
    pub enum RaceStatus {
        Pending,
        Active,
        Finished,
    }

    #[derive(Debug, Clone, scale::Encode, scale::Decode)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo, ink::storage::traits::StorageLayout))]
    pub struct Horse {
        pub id: u8,
        pub position: u32,
        pub finished: bool,
        pub finish_time: Option<u32>,
    }

    #[derive(Debug, Clone, scale::Encode, scale::Decode)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo, ink::storage::traits::StorageLayout))]
    pub struct Race {
        pub id: u8,
        pub status: RaceStatus,
        pub start_block: u32,
        pub current_block: u32,
        pub winner: Option<u8>,
        pub rankings: Vec<u8>,
    }

    #[derive(Debug, Clone, scale::Encode, scale::Decode)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo, ink::storage::traits::StorageLayout))]
    pub struct Bet {
        pub bettor: AccountId,
        pub race_id: u8,
        pub horse_id: u8,
        pub amount: Balance,
    }

    #[ink(storage)]
    pub struct KareraPlatform {
        owner: AccountId,
        races: Mapping<u8, Race>,
        current_race_id: u8,
        // Store horses separately: (race_id, horse_id) -> Horse
        horses: Mapping<(u8, u8), Horse>,
        // Store bets: (bettor, race_id, bet_index) -> Bet
        bet_count: Mapping<(AccountId, u8), u32>,
        bets: Mapping<(AccountId, u8, u32), Bet>,
        total_pool: Mapping<u8, Balance>,
        horse_pools: Mapping<(u8, u8), Balance>,
        payouts_claimed: Mapping<(AccountId, u8), bool>,
    }

    #[ink(event)]
    pub struct RaceStarted {
        #[ink(topic)]
        race_id: u8,
        start_block: u32,
    }

    #[ink(event)]
    pub struct RaceUpdate {
        #[ink(topic)]
        race_id: u8,
        block_number: u32,
        positions: Vec<(u8, u32)>,
    }

    #[ink(event)]
    pub struct RaceFinished {
        #[ink(topic)]
        race_id: u8,
        winner: u8,
        rankings: Vec<u8>,
    }

    #[ink(event)]
    pub struct BetPlaced {
        #[ink(topic)]
        bettor: AccountId,
        race_id: u8,
        horse_id: u8,
        amount: Balance,
    }

    impl KareraPlatform {
        #[ink(constructor)]
        pub fn new() -> Self {
            Self {
                owner: Self::env().caller(),
                races: Mapping::new(),
                current_race_id: 0,
                horses: Mapping::new(),
                bet_count: Mapping::new(),
                bets: Mapping::new(),
                total_pool: Mapping::new(),
                horse_pools: Mapping::new(),
                payouts_claimed: Mapping::new(),
            }
        }

        /// Create a new race
        #[ink(message)]
        pub fn create_race(&mut self) -> Result<u8, Error> {
            if self.current_race_id >= TOTAL_RACES {
                return Err(Error::MaxRacesReached);
            }

            let race_id = self.current_race_id;
            
            // Initialize horses
            for i in 0..HORSES_PER_RACE {
                let horse = Horse {
                    id: i,
                    position: 0,
                    finished: false,
                    finish_time: None,
                };
                self.horses.insert((race_id, i), &horse);
            }

            let race = Race {
                id: race_id,
                status: RaceStatus::Pending,
                start_block: 0,
                current_block: 0,
                winner: None,
                rankings: Vec::new(),
            };

            self.races.insert(race_id, &race);
            self.current_race_id += 1;

            Ok(race_id)
        }

        /// Start a race
        #[ink(message)]
        pub fn start_race(&mut self, race_id: u8) -> Result<(), Error> {
            let caller = self.env().caller();
            if caller != self.owner {
                return Err(Error::Unauthorized);
            }

            let mut race = self.races.get(race_id).ok_or(Error::RaceNotFound)?;
            
            if race.status != RaceStatus::Pending {
                return Err(Error::RaceAlreadyStarted);
            }

            let current_block = self.env().block_number();
            race.status = RaceStatus::Active;
            race.start_block = current_block;
            race.current_block = current_block;

            self.races.insert(race_id, &race);

            self.env().emit_event(RaceStarted {
                race_id,
                start_block: current_block,
            });

            Ok(())
        }

        /// Update race progress
        #[ink(message)]
        pub fn update_race(&mut self, race_id: u8) -> Result<(), Error> {
            let mut race = self.races.get(race_id).ok_or(Error::RaceNotFound)?;
            
            if race.status != RaceStatus::Active {
                return Err(Error::RaceNotActive);
            }

            let current_block = self.env().block_number();
            let blocks_elapsed = current_block - race.start_block;

            // Check if race should end
            if blocks_elapsed >= RACE_DURATION_BLOCKS {
                return self.finish_race(race_id);
            }

            // Update horse positions
            let mut positions = Vec::new();
            let mut all_finished = true;

            for i in 0..HORSES_PER_RACE {
                if let Some(mut horse) = self.horses.get((race_id, i)) {
                    if !horse.finished {
                        // Generate pseudo-random acceleration (15-35 units)
                        let acceleration = self.pseudo_random(horse.id, current_block) % 21 + 15;
                        horse.position += acceleration;

                        // Check if horse finished
                        if horse.position >= FINISH_LINE {
                            horse.position = FINISH_LINE;
                            horse.finished = true;
                            horse.finish_time = Some(blocks_elapsed);
                            race.rankings.push(horse.id);
                        } else {
                            all_finished = false;
                        }

                        positions.push((horse.id, horse.position));
                        self.horses.insert((race_id, i), &horse);
                    }
                }
            }

            race.current_block = current_block;
            self.races.insert(race_id, &race);

            self.env().emit_event(RaceUpdate {
                race_id,
                block_number: current_block,
                positions,
            });

            // If all horses finished, end race
            if all_finished {
                self.finish_race(race_id)?;
            }

            Ok(())
        }

        /// Finish a race
        fn finish_race(&mut self, race_id: u8) -> Result<(), Error> {
            let mut race = self.races.get(race_id).ok_or(Error::RaceNotFound)?;
            
            if race.status != RaceStatus::Active {
                return Err(Error::RaceNotActive);
            }

            // Collect and sort horses
            let mut horses_vec = Vec::new();
            for i in 0..HORSES_PER_RACE {
                if let Some(horse) = self.horses.get((race_id, i)) {
                    horses_vec.push(horse);
                }
            }

            horses_vec.sort_by(|a, b| {
                match (a.finished, b.finished) {
                    (true, true) => a.finish_time.cmp(&b.finish_time),
                    (true, false) => core::cmp::Ordering::Less,
                    (false, true) => core::cmp::Ordering::Greater,
                    (false, false) => b.position.cmp(&a.position),
                }
            });

            race.rankings.clear();
            for horse in horses_vec.iter() {
                race.rankings.push(horse.id);
            }

            race.winner = Some(race.rankings[0]);
            race.status = RaceStatus::Finished;

            self.races.insert(race_id, &race);

            self.env().emit_event(RaceFinished {
                race_id,
                winner: race.rankings[0],
                rankings: race.rankings.clone(),
            });

            Ok(())
        }

        /// Place a bet on a horse
        #[ink(message, payable)]
        pub fn place_bet(&mut self, race_id: u8, horse_id: u8) -> Result<(), Error> {
            let race = self.races.get(race_id).ok_or(Error::RaceNotFound)?;
            
            if race.status != RaceStatus::Pending {
                return Err(Error::BettingClosed);
            }

            if horse_id >= HORSES_PER_RACE {
                return Err(Error::InvalidHorse);
            }

            let bettor = self.env().caller();
            let amount = self.env().transferred_value();

            if amount == 0 {
                return Err(Error::InvalidBetAmount);
            }

            let bet = Bet {
                bettor,
                race_id,
                horse_id,
                amount,
            };

            // Store bet
            let count = self.bet_count.get((bettor, race_id)).unwrap_or(0);
            self.bets.insert((bettor, race_id, count), &bet);
            self.bet_count.insert((bettor, race_id), &(count + 1));

            // Update pools
            let total = self.total_pool.get(race_id).unwrap_or(0);
            self.total_pool.insert(race_id, &(total + amount));

            let horse_total = self.horse_pools.get((race_id, horse_id)).unwrap_or(0);
            self.horse_pools.insert((race_id, horse_id), &(horse_total + amount));

            self.env().emit_event(BetPlaced {
                bettor,
                race_id,
                horse_id,
                amount,
            });

            Ok(())
        }

        /// Claim winnings
        #[ink(message)]
        pub fn claim_winnings(&mut self, race_id: u8) -> Result<Balance, Error> {
            let race = self.races.get(race_id).ok_or(Error::RaceNotFound)?;
            
            if race.status != RaceStatus::Finished {
                return Err(Error::RaceNotFinished);
            }

            let caller = self.env().caller();
            let claimed = self.payouts_claimed.get((caller, race_id)).unwrap_or(false);
            
            if claimed {
                return Err(Error::AlreadyClaimed);
            }

            let winner_horse = race.winner.ok_or(Error::NoWinner)?;
            
            // Calculate total bet on winning horse
            let bet_count = self.bet_count.get((caller, race_id)).unwrap_or(0);
            let mut total_bet = 0u128;
            
            for i in 0..bet_count {
                if let Some(bet) = self.bets.get((caller, race_id, i)) {
                    if bet.horse_id == winner_horse {
                        total_bet += bet.amount;
                    }
                }
            }

            if total_bet == 0 {
                return Err(Error::NoWinningBets);
            }

            let total_pool = self.total_pool.get(race_id).unwrap_or(0);
            let winning_pool = self.horse_pools.get((race_id, winner_horse)).unwrap_or(0);

            if winning_pool == 0 {
                return Err(Error::NoWinningBets);
            }

            // Payout = (user_bet / winning_pool) * total_pool
            let payout = (total_bet * total_pool) / winning_pool;

            // Mark as claimed
            self.payouts_claimed.insert((caller, race_id), &true);

            // Transfer winnings
            if self.env().transfer(caller, payout).is_err() {
                return Err(Error::TransferFailed);
            }

            Ok(payout)
        }

        /// Get race details
        #[ink(message)]
        pub fn get_race(&self, race_id: u8) -> Option<Race> {
            self.races.get(race_id)
        }

        /// Get horse details
        #[ink(message)]
        pub fn get_horse(&self, race_id: u8, horse_id: u8) -> Option<Horse> {
            self.horses.get((race_id, horse_id))
        }

        /// Get all horses for a race
        #[ink(message)]
        pub fn get_all_horses(&self, race_id: u8) -> Vec<Horse> {
            let mut horses = Vec::new();
            for i in 0..HORSES_PER_RACE {
                if let Some(horse) = self.horses.get((race_id, i)) {
                    horses.push(horse);
                }
            }
            horses
        }

        /// Get current race count
        #[ink(message)]
        pub fn get_race_count(&self) -> u8 {
            self.current_race_id
        }

        /// Get total pool for a race
        #[ink(message)]
        pub fn get_total_pool(&self, race_id: u8) -> Balance {
            self.total_pool.get(race_id).unwrap_or(0)
        }

        /// Get horse pool
        #[ink(message)]
        pub fn get_horse_pool(&self, race_id: u8, horse_id: u8) -> Balance {
            self.horse_pools.get((race_id, horse_id)).unwrap_or(0)
        }

        /// Pseudo-random number generator
        fn pseudo_random(&self, seed: u8, block: u32) -> u32 {
            let hash = self.env().block_timestamp() as u32 
                ^ block 
                ^ (seed as u32);
            
            // Simple LCG algorithm
            let a: u32 = 1664525;
            let c: u32 = 1013904223;
            (a.wrapping_mul(hash).wrapping_add(c)) % 100
        }
    }

    #[derive(Debug, PartialEq, Eq, scale::Encode, scale::Decode)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub enum Error {
        RaceNotFound,
        RaceNotActive,
        RaceAlreadyStarted,
        RaceNotFinished,
        MaxRacesReached,
        InvalidHorse,
        BettingClosed,
        InvalidBetAmount,
        NoWinner,
        NoWinningBets,
        AlreadyClaimed,
        TransferFailed,
        Unauthorized,
    }
}