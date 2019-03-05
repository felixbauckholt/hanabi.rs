use fnv::{FnvHashMap, FnvHashSet};
use std::cmp::Ordering;

use strategy::*;
use game::*;
use helpers::*;
use strategies::hat_helpers::*;

// TODO: use random extra information - i.e. when casting up and down,
// we sometimes have 2 choices of value to choose
// TODO: guess very aggressively at very end of game (first, see whether
// situation ever occurs)

type PropertyPredicate = fn(&BoardState, &Card) -> bool;

struct CardHasProperty
{
    index: usize,
    property: PropertyPredicate,
}
impl Question for CardHasProperty
{
    fn info_amount(&self) -> u32 { 2 }
    fn answer(&self, hand: &Cards, board: &BoardState) -> u32 {
        let ref card = hand[self.index];
        if (self.property)(board, card) { 1 } else { 0 }
    }
    fn acknowledge_answer(
        &self,
        answer: u32,
        hand_info: &mut HandInfo<CardPossibilityTable>,
        board: &BoardState,
    ) {
        let ref mut card_table = hand_info[self.index];
        let possible = card_table.get_possibilities();
        for card in &possible {
            if (self.property)(board, card) {
                if answer == 0 { card_table.mark_false(card); }
            } else {
                if answer == 1 { card_table.mark_false(card); }
            }
        }
    }
}
fn q_is_playable(index: usize) -> CardHasProperty {
    CardHasProperty {index, property: |board, card| board.is_playable(card)}
}
fn q_is_dead(index: usize) -> CardHasProperty {
    CardHasProperty {index, property: |board, card| board.is_dead(card)}
}

/// For some list of questions l, the question `AdditiveComboQuestion { questions : l }` asks:
/// "What is the first question in the list `l` that has a nonzero answer, and what is its
/// answer?"
/// If all questions in `l` have the answer `0`, this question has the answer `0` as well.
///
/// It's named that way because the `info_amount` grows additively with the `info_amount`s of
/// the questions in `l`.
struct AdditiveComboQuestion {
    questions: Vec<Box<Question>>,
}
impl Question for AdditiveComboQuestion {
    fn info_amount(&self) -> u32 {
        self.questions.iter().map(|q| { q.info_amount() - 1 }).sum::<u32>() + 1
    }
    fn answer(&self, hand: &Cards, board: &BoardState) -> u32 {
        let mut toadd = 1;
        for q in &self.questions {
            let q_answer = q.answer(hand, board);
            if q_answer != 0 {
                return toadd + q_answer - 1;
            }
            toadd += q.info_amount() - 1;
        }
        assert!(toadd == self.info_amount());
        0
    }
    fn acknowledge_answer(
        &self,
        mut answer: u32,
        hand_info: &mut HandInfo<CardPossibilityTable>,
        board: &BoardState,
    ) {
        if answer == 0 {
            answer = self.info_amount();
        }
        answer -= 1;
        for q in &self.questions {
            if answer < q.info_amount() - 1 {
                q.acknowledge_answer(answer+1, hand_info, board);
                return;
            } else {
                q.acknowledge_answer(0, hand_info, board);
                answer -= q.info_amount() - 1;
            }
        }
        assert!(answer == 0);
    }
}

#[derive(Debug)]
struct CardPossibilityPartition {
    index: usize,
    n_partitions: u32,
    partition: FnvHashMap<Card, u32>,
}
impl CardPossibilityPartition {
    fn new(
        index: usize, max_n_partitions: u32, card_table: &CardPossibilityTable, board: &BoardState
    ) -> CardPossibilityPartition {
        let mut cur_block = 0;
        let mut partition = FnvHashMap::default();
        let mut n_partitions = 0;

        let has_dead = card_table.probability_is_dead(&board) != 0.0;

        // TODO: group things of different colors and values?
        let mut effective_max = max_n_partitions;
        if has_dead {
            effective_max -= 1;
        };

        for card in card_table.get_possibilities() {
            if !board.is_dead(&card) {
                partition.insert(card.clone(), cur_block);
                cur_block = (cur_block + 1) % effective_max;
                if n_partitions < effective_max {
                    n_partitions += 1;
                }
            }
        }

        if has_dead {
            for card in card_table.get_possibilities() {
                if board.is_dead(&card) {
                    partition.insert(card.clone(), n_partitions);
                }
            }
            n_partitions += 1;
        }

        // let mut s : String = "Partition: |".to_string();
        // for i in 0..n_partitions {
        //     for (card, block) in partition.iter() {
        //         if *block == i {
        //             s = s + &format!(" {}", card);
        //         }
        //     }
        //     s = s + &format!(" |");
        // }
        // debug!("{}", s);

        CardPossibilityPartition {
            index: index,
            n_partitions: n_partitions,
            partition: partition,
        }
    }
}
impl Question for CardPossibilityPartition {
    fn info_amount(&self) -> u32 { self.n_partitions }
    fn answer(&self, hand: &Cards, _: &BoardState) -> u32 {
        let ref card = hand[self.index];
        *self.partition.get(&card).unwrap()
    }
    fn acknowledge_answer(
        &self,
        answer: u32,
        hand_info: &mut HandInfo<CardPossibilityTable>,
        _: &BoardState,
    ) {
        let ref mut card_table = hand_info[self.index];
        let possible = card_table.get_possibilities();
        for card in &possible {
            if *self.partition.get(card).unwrap() != answer {
                card_table.mark_false(card);
            }
        }
    }
}

#[derive(Eq,PartialEq,Clone)]
struct MyPublicInformation {
    hand_info: FnvHashMap<Player, HandInfo<CardPossibilityTable>>,
    card_counts: CardCounts, // what any newly drawn card should be
    board: BoardState, // TODO: maybe we should store an appropriately lifetimed reference?
}

impl MyPublicInformation {
    fn get_player_info_mut(&mut self, player: &Player) -> &mut HandInfo<CardPossibilityTable> {
        self.hand_info.get_mut(player).unwrap()
    }
    fn take_player_info(&mut self, player: &Player) -> HandInfo<CardPossibilityTable> {
        self.hand_info.remove(player).unwrap()
    }

    fn get_other_players_starting_after(&self, player: Player) -> Vec<Player> {
        let n = self.board.num_players;
        (0 .. n - 1).into_iter().map(|i| { (player + 1 + i) % n }).collect()
    }

    // Returns the number of ways to hint the player.
    fn get_info_per_player(&self, player: Player) -> u32 {
        // Determine if both:
        //  - it is public that there are at least two colors
        //  - it is public that there are at least two numbers

        let ref info = self.hand_info[&player];

        let may_be_all_one_color = COLORS.iter().any(|color| {
            info.iter().all(|card| {
                card.can_be_color(*color)
            })
        });

        let may_be_all_one_number = VALUES.iter().any(|value| {
            info.iter().all(|card| {
                card.can_be_value(*value)
            })
        });

        return if !may_be_all_one_color && !may_be_all_one_number { 4 } else { 3 }
    }

    fn get_hint_index_score(&self, card_table: &CardPossibilityTable) -> i32 {
        if card_table.probability_is_dead(&self.board) == 1.0 {
            return 0;
        }
        if card_table.is_determined() {
            return 0;
        }
        // Do something more intelligent?
        let mut score = 1;
        if !card_table.color_determined() {
            score += 1;
        }
        if !card_table.value_determined() {
            score += 1;
        }
        return score;
    }

    fn get_index_for_hint(&self, player: &Player) -> usize {
        let mut scores = self.hand_info[player].iter().enumerate().map(|(i, card_table)| {
            let score = self.get_hint_index_score(card_table);
            (-score, i)
        }).collect::<Vec<_>>();
        scores.sort();
        scores[0].1
    }

    // TODO: refactor out the common parts of get_hint and update_from_hint_choice
    fn get_hint(&mut self, view: &OwnedGameView) -> Vec<Hint> {
        // Can give up to 3(n-1) hints
        // For any given player with at least 4 cards, and index i, there are at least 3 hints that can be given.
        // 0. a value hint on card i
        // 1. a color hint on card i
        // 2. any hint not involving card i
        // However, if it is public info that the player has at least two colors
        // and at least two numbers, then instead we do
        // 2. any color hint not involving i
        // 3. any color hint not involving i

        // TODO: make it so space of hints is larger when there is
        // knowledge about the cards?

        let hinter = view.player;
        let info_per_player: Vec<_> = self.get_other_players_starting_after(hinter).into_iter().map(
            |player| { self.get_info_per_player(player) }
        ).collect();
        let total_info = info_per_player.iter().sum();
        // FIXME explain and clean up
        let card_indices: Vec<_> = self.get_other_players_starting_after(hinter).into_iter().map(
            |player| { self.get_index_for_hint(&player) }
        ).collect();

        let hint_info = self.get_hat_sum(total_info, view);

        //let hint_type = hint_info.value % 3;
        //let player_amt = (hint_info.value - hint_type) / 3;
        let mut hint_type = hint_info.value;
        let mut player_amt = 0;
        while hint_type >= info_per_player[player_amt] {
            hint_type -= info_per_player[player_amt];
            player_amt += 1;
        }
        let hint_info_we_can_give_to_this_player = info_per_player[player_amt];

        let hint_player = (hinter + 1 + (player_amt as u32)) % view.board.num_players;

        let hand = view.get_hand(&hint_player);
        let card_index = card_indices[player_amt];
        let hint_card = &hand[card_index];

        let hint_option_set = if hint_info_we_can_give_to_this_player == 3 {
            match hint_type {
                0 => {
                    vec![Hinted::Value(hint_card.value)]
                }
                1 => {
                    vec![Hinted::Color(hint_card.color)]
                }
                2 => {
                    // NOTE: this doesn't do that much better than just hinting
                    // the first thing that doesn't match the hint_card
                    let mut hint_option_set = Vec::new();
                    for card in hand {
                        if card.color != hint_card.color {
                            hint_option_set.push(Hinted::Color(card.color));
                        }
                        if card.value != hint_card.value {
                            hint_option_set.push(Hinted::Value(card.value));
                        }
                    }
                    hint_option_set
                }
                _ => {
                    panic!("Invalid hint type")
                }
            }
        } else {
            match hint_type {
                0 => {
                    vec![Hinted::Value(hint_card.value)]
                }
                1 => {
                    vec![Hinted::Color(hint_card.color)]
                }
                2 => {
                    // Any value hint for a card other than the first
                    let mut hint_option_set = Vec::new();
                    for card in hand {
                        if card.value != hint_card.value {
                            hint_option_set.push(Hinted::Value(card.value));
                        }
                    }
                    hint_option_set
                }
                3 => {
                    // Any color hint for a card other than the first
                    let mut hint_option_set = Vec::new();
                    for card in hand {
                        if card.color != hint_card.color {
                            hint_option_set.push(Hinted::Color(card.color));
                        }
                    }
                    hint_option_set
                }
                _ => {
                    panic!("Invalid hint type")
                }
            }
        };
        hint_option_set.into_iter().collect::<FnvHashSet<_>>().into_iter().map(|hinted| {
            Hint {
                player: hint_player,
                hinted: hinted,
            }
        }).collect()
    }

    fn decode_hint_choice(&self, hint: &Hint, result: &Vec<bool>) -> ModulusInformation {
        let hinter = self.board.player;

        let info_per_player: Vec<_> = self.get_other_players_starting_after(hinter).into_iter().map(
            |player| { self.get_info_per_player(player)  }
        ).collect();
        let total_info = info_per_player.iter().sum();

        let n = self.board.num_players;

        let player_amt = (n + hint.player - hinter - 1) % n;

        let amt_from_prev_players = info_per_player.iter().take(player_amt as usize).fold(0, |a, b| a + b);
        let hint_info_we_can_give_to_this_player = info_per_player[player_amt as usize];

        let card_index = self.get_index_for_hint(&hint.player);
        let hint_type =
            if hint_info_we_can_give_to_this_player == 3 {
                if result[card_index] {
                    match hint.hinted {
                        Hinted::Value(_) => 0,
                        Hinted::Color(_) => 1,
                    }
                } else {
                    2
                }
            } else {
                if result[card_index] {
                    match hint.hinted {
                        Hinted::Value(_) => 0,
                        Hinted::Color(_) => 1,
                    }
                } else {
                    match hint.hinted {
                        Hinted::Value(_) => 2,
                        Hinted::Color(_) => 3,
                    }
                }
            };

        let hint_value = amt_from_prev_players + hint_type;

        ModulusInformation::new(total_info, hint_value)
    }

    fn update_from_hint_choice(&mut self, hint: &Hint, matches: &Vec<bool>, view: &OwnedGameView) {
        let info = self.decode_hint_choice(hint, matches);
        self.update_from_hat_sum(info, view);
    }

    fn update_from_hint_matches(&mut self, hint: &Hint, matches: &Vec<bool>) {
        let info = self.get_player_info_mut(&hint.player);
        info.update_for_hint(&hint.hinted, matches);
    }

    fn knows_playable_card(&self, player: &Player) -> bool {
            self.hand_info[player].iter().any(|table| {
                table.probability_is_playable(&self.board) == 1.0
            })
    }

    fn knows_dead_card(&self, player: &Player) -> bool {
            self.hand_info[player].iter().any(|table| {
                table.probability_is_dead(&self.board) == 1.0
            })
    }

    fn someone_else_needs_hint(&self, view: &OwnedGameView) -> bool {
        // Does another player have a playable card, but doesn't know it?
        view.get_other_players().iter().any(|player| {
            let has_playable_card = view.get_hand(&player).iter().any(|card| {
                view.get_board().is_playable(card)
            });
            has_playable_card && !self.knows_playable_card(&player)
        })
    }

    fn update_noone_else_needs_hint(&mut self) {
        // If it becomes public knowledge that someone_else_needs_hint() returns false,
        // update accordingly.
        for player in self.board.get_players() {
            if player != self.board.player && !self.knows_playable_card(&player) {
                // If player doesn't know any playable cards, player doesn't have any playable
                // cards.
                let mut hand_info = self.take_player_info(&player);
                for ref mut card_table in hand_info.iter_mut() {
                    let possible = card_table.get_possibilities();
                    for card in &possible {
                        if self.board.is_playable(card) {
                            card_table.mark_false(card);
                        }
                    }
                }
                self.set_player_info(&player, hand_info);
            }
        }
    }

    fn update_from_discard_or_play_result(
        &mut self,
        new_view: &BorrowedGameView,
        player: &Player,
        index: usize,
        card: &Card
    ) {
        let new_card_table = CardPossibilityTable::from(&self.card_counts);
        {
            let info = self.get_player_info_mut(player);
            assert!(info[index].is_possible(card));
            info.remove(index);

            // push *before* incrementing public counts
            if info.len() < new_view.hand_size(&player) {
                info.push(new_card_table);
            }
        }

        // TODO: decrement weight counts for fully determined cards, ahead of time

        for player in self.board.get_players() {
            let info = self.get_player_info_mut(&player);
            for card_table in info.iter_mut() {
                card_table.decrement_weight_if_possible(card);
            }
        }

        self.card_counts.increment(card);
    }
}

impl PublicInformation for MyPublicInformation {
    fn new(board: &BoardState) -> Self {
        let hand_info = board.get_players().map(|player| {
            let hand_info = HandInfo::new(board.hand_size);
            (player, hand_info)
        }).collect::<FnvHashMap<_,_>>();
        MyPublicInformation {
            hand_info: hand_info,
            card_counts: CardCounts::new(),
            board: board.clone(),
        }
    }

    fn set_board(&mut self, board: &BoardState) {
        self.board = board.clone();
    }

    fn get_player_info(&self, player: &Player) -> HandInfo<CardPossibilityTable> {
        self.hand_info[player].clone()
    }

    fn set_player_info(&mut self, player: &Player, hand_info: HandInfo<CardPossibilityTable>) {
        self.hand_info.insert(*player, hand_info);
    }

    fn agrees_with(&self, other: Self) -> bool {
        *self == other
    }

    fn ask_questions<Callback>(
        &self,
        me: &Player,
        hand_info: &mut HandInfo<CardPossibilityTable>,
        mut ask_question: Callback,
        mut info_remaining: u32,
    ) where Callback: FnMut(&mut HandInfo<CardPossibilityTable>, &mut u32, Box<Question>) {
        // Changing anything inside this function will not break the information transfer
        // mechanisms!

        let compute_augmented_hand_info = |hand_info: &HandInfo<CardPossibilityTable>| {
            hand_info.iter().cloned().enumerate()
            .map(|(i, card_table)| {
                let p_play = card_table.probability_is_playable(&self.board);
                let p_dead = card_table.probability_is_dead(&self.board);
                let is_determined = card_table.is_determined();
                (card_table, i, p_play, p_dead, is_determined)
            })
            .collect::<Vec<_>>()
        };

        if !self.knows_playable_card(me) { // TODO: changing this to "if true {" slightly improves the three-player game and
                                           // very slightly worsens the other cases. There probably is some
                                           // other way to make this decision that's better in all cases.
            let augmented_hand_info = compute_augmented_hand_info(hand_info);
            let mut ask_play = augmented_hand_info.iter()
                .filter(|&&(_, _, p_play, p_dead, is_determined)| {
                    if is_determined { return false; }
                    if p_dead == 1.0  { return false; }
                    if p_play == 1.0 || p_play < 0.2 { return false; }
                    true
                }).collect::<Vec<_>>();
            // sort by probability of play, then by index
            ask_play.sort_by(|&&(_, i1, p1, _, _), &&(_, i2, p2, _, _)| {
                    // It's better to include higher-probability-of-playability
                    // cards into our combo question, since that maximizes our
                    // chance of finding out about a playable card.
                    let result = p2.partial_cmp(&p1);
                    if result == None || result == Some(Ordering::Equal) {
                        i1.cmp(&i2)
                    } else {
                        result.unwrap()
                    }
            });

            if self.board.num_players == 5 {
                for &(_, i, _, _, _) in ask_play {
                    ask_question(hand_info, &mut info_remaining, Box::new(q_is_playable(i)));
                    if info_remaining <= 1 { return; }
                }
            } else {
                let mut rest_combo = AdditiveComboQuestion {questions: Vec::new()};
                for &(_, i, _, _, _) in ask_play {
                    if rest_combo.info_amount() < info_remaining {
                        rest_combo.questions.push(Box::new(q_is_playable(i)));
                    }
                }
                rest_combo.questions.reverse(); // It's better to put lower-probability-of-playability
                                                // cards first: The difference only matters if we
                                                // find a playable card, and conditional on that,
                                                // it's better to find out about as many non-playable
                                                // cards as possible.
                if rest_combo.info_amount() < info_remaining && !self.knows_dead_card(me) {
                    let mut ask_dead = augmented_hand_info.iter()
                        .filter(|&&(_, _, _, p_dead, _)| {
                            p_dead > 0.0 && p_dead < 1.0
                        }).collect::<Vec<_>>();
                    // sort by probability of death, then by index
                    ask_dead.sort_by(|&&(_, i1, _, d1, _), &&(_, i2, _, d2, _)| {
                            let result = d2.partial_cmp(&d1);
                            if result == None || result == Some(Ordering::Equal) {
                                i1.cmp(&i2)
                            } else {
                                result.unwrap()
                            }
                    });
                    for &(_, i, _, _, _) in ask_dead {
                        if rest_combo.info_amount() < info_remaining {
                            rest_combo.questions.push(Box::new(q_is_dead(i)));
                        }
                    }
                }
                ask_question(hand_info, &mut info_remaining, Box::new(rest_combo));
                if info_remaining <= 1 { return; }
            }
        }

        // Recompute augmented_hand_info, incorporating the things we learned when asking questions
        let augmented_hand_info = compute_augmented_hand_info(hand_info);
        let mut ask_partition = augmented_hand_info.iter()
            .filter(|&&(_, _, _, p_dead, is_determined)| {
                if is_determined { return false }
                // TODO: possibly still valuable to ask?
                if p_dead == 1.0 { return false }
                true
            }).collect::<Vec<_>>();
        // sort by probability of play, then by index
        ask_partition.sort_by(|&&(_, i1, p1, _, _), &&(_, i2, p2, _, _)| {
                // *higher* probabilities are better
                let result = p2.partial_cmp(&p1);
                if result == None || result == Some(Ordering::Equal) {
                    i1.cmp(&i2)
                } else {
                    result.unwrap()
                }
        });

        for &(ref card_table, i, _, _, _) in ask_partition {
            let question = CardPossibilityPartition::new(i, info_remaining, &card_table, &self.board);
            ask_question(hand_info, &mut info_remaining, Box::new(question));
            if info_remaining <= 1 { return; }
        }
    }
}



pub struct InformationStrategyConfig;

impl InformationStrategyConfig {
    pub fn new() -> InformationStrategyConfig {
        InformationStrategyConfig
    }
}
impl GameStrategyConfig for InformationStrategyConfig {
    fn initialize(&self, _: &GameOptions) -> Box<GameStrategy> {
        Box::new(InformationStrategy::new())
    }
}

pub struct InformationStrategy;

impl InformationStrategy {
    pub fn new() -> InformationStrategy {
        InformationStrategy
    }
}
impl GameStrategy for InformationStrategy {
    fn initialize(&self, player: Player, view: &BorrowedGameView) -> Box<PlayerStrategy> {
        Box::new(InformationPlayerStrategy {
            me: player,
            public_info: MyPublicInformation::new(view.board),
            new_public_info: None,
            last_view: OwnedGameView::clone_from(view),
        })
    }
}

pub struct InformationPlayerStrategy {
    me: Player,
    public_info: MyPublicInformation,
    // Inside decide(), modify a copy of public_info and put it here. After that, when
    // calling update, check that the updated public_info matches new_public_info.
    new_public_info: Option<MyPublicInformation>,
    last_view: OwnedGameView, // the view on the previous turn
}

impl InformationPlayerStrategy {
    // how badly do we need to play a particular card
    fn get_average_play_score(&self, view: &OwnedGameView, card_table: &CardPossibilityTable) -> f32 {
        let f = |card: &Card| { self.get_play_score(view, card) };
        card_table.weighted_score(&f)
    }

    fn get_play_score(&self, view: &OwnedGameView, card: &Card) -> f32 {
        let mut num_with = 1;
        if view.board.deck_size > 0 {
            for player in view.board.get_players() {
                if player != self.me {
                    if view.has_card(&player, card) {
                        num_with += 1;
                    }
                }
            }
        }
        (10.0 - card.value as f32) / (num_with as f32)
    }

    fn find_useless_cards(&self, board: &BoardState, hand: &HandInfo<CardPossibilityTable>) -> Vec<usize> {
        let mut useless: FnvHashSet<usize> = FnvHashSet::default();
        let mut seen: FnvHashMap<Card, usize> = FnvHashMap::default();

        for (i, card_table) in hand.iter().enumerate() {
            if card_table.probability_is_dead(board) == 1.0 {
                useless.insert(i);
            } else {
                if let Some(card) = card_table.get_card() {
                    if seen.contains_key(&card) {
                        // found a duplicate card
                        useless.insert(i);
                        useless.insert(*seen.get(&card).unwrap());
                    } else {
                        seen.insert(card, i);
                    }
                }
            }
        }
        let mut useless_vec : Vec<usize> = useless.into_iter().collect();
        useless_vec.sort();
        return useless_vec;
    }

    // how good is it to give this hint to this player?
    fn hint_goodness(&self, hint: &Hint, view: &OwnedGameView) -> f32 {
        // This gets called after self.public_info.get_hint(), which modifies the public
        // info to include information gained through question answering. Therefore, we only
        // simulate information gained through the hint result here.

        let hint_player = &hint.player;
        let hinted = &hint.hinted;
        let hand = view.get_hand(&hint_player);
        let mut hand_info = self.public_info.get_player_info(&hint_player);

        let mut goodness = 1.0;
        for (i, card_table) in hand_info.iter_mut().enumerate() {
            let card = &hand[i];
            if card_table.probability_is_dead(&view.board) == 1.0 {
                continue;
            }
            if card_table.is_determined() {
                continue;
            }
            let old_weight = card_table.total_weight();
            match *hinted {
                Hinted::Color(color) => {
                    card_table.mark_color(color, color == card.color)
                }
                Hinted::Value(value) => {
                    card_table.mark_value(value, value == card.value)
                }
            };
            let new_weight = card_table.total_weight();
            assert!(new_weight <= old_weight);
            let bonus = {
                if card_table.is_determined() {
                    2
                } else if card_table.probability_is_dead(&view.board) == 1.0 {
                    2
                } else {
                    1
                }
            };
            goodness *= (bonus as f32) * (old_weight / new_weight);
        }
        goodness
    }

    fn get_best_hint_of_options(&self, mut hints: Vec<Hint>) -> Hint {
        if hints.len() == 1 {
            return hints.remove(0);
        }
        let view = &self.last_view;

        // using hint goodness barely helps
        let mut hint_options = hints.into_iter().map(|hint| {
            (self.hint_goodness(&hint, view), hint)
        }).collect::<Vec<_>>();

        hint_options.sort_by(|h1, h2| {
            h2.0.partial_cmp(&h1.0).unwrap_or(Ordering::Equal)
        });

        if hint_options.len() == 0 {
            // NOTE: Technically possible, but never happens
        } else {
            if hint_options.len() > 1 {
                debug!("Choosing amongst hint options: {:?}", hint_options);
            }
        }
        hint_options.remove(0).1
    }

    /// Decide on a move. At the same time, simulate the impact of that move on the public
    /// information state by modifying `public_info`. Since `self` is immutable and since our
    /// public information state change will be compared against the change in the corresponding
    /// call to `update_wrapped`, nothing we do here will let our public information state silently
    /// get out of sync with other players' public information state!
    fn decide_wrapped(&mut self, public_info: &mut MyPublicInformation) -> TurnChoice {
        // we already stored the view
        let view = &self.last_view;
        let me = &view.player;

        for player in view.board.get_players() {
           let hand_info = public_info.get_player_info(&player);
            debug!("Current state of hand_info for {}:", player);
            for (i, card_table) in hand_info.iter().enumerate() {
                debug!("  Card {}: {}", i, card_table);
            }
        }

        let private_info = public_info.get_private_info(view);
        // debug!("My info:");
        // for (i, card_table) in private_info.iter().enumerate() {
        //     debug!("{}: {}", i, card_table);
        // }

        let playable_cards = private_info.iter().enumerate().filter(|&(_, card_table)| {
            card_table.probability_is_playable(&view.board) == 1.0
        }).collect::<Vec<_>>();

        if playable_cards.len() > 0 {
            // play the best playable card
            // the higher the play_score, the better to play
            let mut play_score = -1.0;
            let mut play_index = 0;

            for (index, card_table) in playable_cards {
                let score = self.get_average_play_score(view, card_table);
                if score > play_score {
                    play_score = score;
                    play_index = index;
                }
            }

            return TurnChoice::Play(play_index)
        }

        let discard_threshold =
            view.board.total_cards
            - (COLORS.len() * VALUES.len()) as u32
            - (view.board.num_players * view.board.hand_size);

        // make a possibly risky play
        // TODO: consider removing this, if we improve information transfer
        if view.board.lives_remaining > 1 &&
           view.board.discard_size() <= discard_threshold
        {
            let mut risky_playable_cards = private_info.iter().enumerate().filter(|&(_, card_table)| {
                // card is either playable or dead
                card_table.probability_of_predicate(&|card| {
                    view.board.is_playable(card) || view.board.is_dead(card)
                }) == 1.0
            }).map(|(i, card_table)| {
                let p = card_table.probability_is_playable(&view.board);
                (i, card_table, p)
            }).collect::<Vec<_>>();

            if risky_playable_cards.len() > 0 {
                risky_playable_cards.sort_by(|c1, c2| {
                    c2.2.partial_cmp(&c1.2).unwrap_or(Ordering::Equal)
                });

                let maybe_play = risky_playable_cards[0];
                if maybe_play.2 > 0.75 {
                    return TurnChoice::Play(maybe_play.0);
                }
            }
        }

        let public_useless_indices = self.find_useless_cards(&view.board, &public_info.get_player_info(me));
        let useless_indices = self.find_useless_cards(&view.board, &private_info);
        let private_useless_index = useless_indices.iter().find(|&index| {
            !public_useless_indices.iter().any(|other_index| other_index == index)
        });

        // NOTE When changing this, make sure to keep the "discard" branch of update() up to date!
        let will_hint =
            if view.board.hints_remaining > 0 && public_info.someone_else_needs_hint(view) { true }
            else if view.board.discard_size() <= discard_threshold && useless_indices.len() > 0 { false }
            // hinting is better than discarding dead cards
            // (probably because it stalls the deck-drawing).
            else if view.board.hints_remaining > 0 && view.someone_else_can_play() { true }
            else if view.board.hints_remaining > 4 { true }
            // this is the only case in which we discard a potentially useful card.
            else { false };

        if will_hint {
            let hint_set = public_info.get_hint(view);
            let hint = self.get_best_hint_of_options(hint_set);
            return TurnChoice::Hint(hint);
        }

        if self.last_view.board.hints_remaining > 0 {
            public_info.update_noone_else_needs_hint();
        }

        // if anything is totally useless, discard it
        if public_useless_indices.len() > 0 {
            let info_amt = public_useless_indices.len() as u32;
            if let Some(private_useless_index) = private_useless_index {
                let num_states = info_amt + 2;
                if public_info.decide_action_not_known_to_be_possible(num_states, view) {
                    return TurnChoice::Discard(*private_useless_index);
                }
            }
            let info = public_info.get_hat_sum(info_amt, view);
            return TurnChoice::Discard(public_useless_indices[info.value as usize]);
        } else if useless_indices.len() > 0 {
            return TurnChoice::Discard(useless_indices[0]);
        }

        // Play the best discardable card
        let mut compval = 0.0;
        let mut index = 0;
        for (i, card_table) in private_info.iter().enumerate() {
            let probability_is_seen = card_table.probability_of_predicate(&|card| {
                view.can_see(card)
            });
            let my_compval =
                20.0 * probability_is_seen
                + 10.0 * card_table.probability_is_dispensable(&view.board)
                + card_table.average_value();

            if my_compval > compval {
                compval = my_compval;
                index = i;
            }
        }
        TurnChoice::Discard(index)
    }

    /// Update the public information. The "update" operations on the public information state have to
    /// exactly match the corresponding "choice" operations in `decide_wrapped()`.
    ///
    /// We don't have to update on the "turn result" here. If the turn was a hint, we get the
    /// matches in order to understand the "intention" behind the hint, but we do not need to
    /// update on what the hint says about the hinted player's cards directly. (This is done in the
    /// call to `update_hint_matches()` inside `update()`.
    fn update_wrapped(
        &mut self,
        turn_player: &Player,
        turn_choice: &TurnChoice,
        hint_matches: Option<&Vec<bool>>,
    ) {
        match turn_choice {
            TurnChoice::Hint(ref hint) =>  {
                let matches = hint_matches.unwrap();
                self.public_info.update_from_hint_choice(hint, matches, &self.last_view);
            }
            TurnChoice::Discard(index) => {
                if self.last_view.board.hints_remaining > 0 {
                    self.public_info.update_noone_else_needs_hint();
                }

                let known_useless_indices = self.find_useless_cards(
                    &self.last_view.board, &self.public_info.get_player_info(turn_player)
                );
                if known_useless_indices.len() > 0 {
                    let info_amt = known_useless_indices.len() as u32;
                    if let Some(value) = known_useless_indices.iter().position(|&i| i == *index) {
                        let info = ModulusInformation::new(info_amt, value as u32);
                        self.public_info.update_from_hat_sum(info, &self.last_view);
                    } else {
                        // The player discarded a card they privately knew to be dead!
                        let num_states = info_amt + 2;
                        self.public_info.update_from_action_not_known_to_be_possible(num_states, &self.last_view);
                    }
                }
            }
            TurnChoice::Play(_index) => {
                // TODO: Maybe we can transfer information through plays as well?
            }
        }
    }
}

impl PlayerStrategy for InformationPlayerStrategy {
    fn decide(&mut self, _: &BorrowedGameView) -> TurnChoice {
        let mut public_info = self.public_info.clone();
        let turn_choice = self.decide_wrapped(&mut public_info);
        self.new_public_info = Some(public_info);
        turn_choice
    }

    fn update(&mut self, turn_record: &TurnRecord, view: &BorrowedGameView) {
        let hint_matches = if let &TurnResult::Hint(ref matches) = &turn_record.result {
            Some(matches)
        } else { None };
        self.update_wrapped(&turn_record.player, &turn_record.choice, hint_matches);
        if let Some(new_public_info) = self.new_public_info.take() {
            if !self.public_info.agrees_with(new_public_info) {
                panic!("The change made to public_info in self.decide_wrapped differs from \
                        the corresponding change in self.update_wrapped!");
            }
        }
        match turn_record.choice {
            TurnChoice::Hint(ref hint) =>  {
                if let &TurnResult::Hint(ref matches) = &turn_record.result {
                    self.public_info.update_from_hint_matches(hint, matches);
                } else {
                    panic!("Got turn choice {:?}, but turn result {:?}",
                           turn_record.choice, turn_record.result);
                }
            }
            TurnChoice::Discard(index) => {
                if let &TurnResult::Discard(ref card) = &turn_record.result {
                    self.public_info.update_from_discard_or_play_result(view, &turn_record.player, index, card);
                } else {
                    panic!("Got turn choice {:?}, but turn result {:?}",
                           turn_record.choice, turn_record.result);
                }
            }
            TurnChoice::Play(index) =>  {
                if let &TurnResult::Play(ref card, _) = &turn_record.result {
                    self.public_info.update_from_discard_or_play_result(view, &turn_record.player, index, card);
                } else {
                    panic!("Got turn choice {:?}, but turn result {:?}",
                           turn_record.choice, turn_record.result);
                }
            }
        }
        self.last_view = OwnedGameView::clone_from(view);
        self.public_info.set_board(view.board);
    }
}
