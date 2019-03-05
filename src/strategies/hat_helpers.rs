use game::*;
use helpers::*;

#[derive(Debug,Clone)]
pub struct ModulusInformation {
    pub modulus: u32,
    pub value: u32,
}
impl ModulusInformation {
    pub fn new(modulus: u32, value: u32) -> Self {
        assert!(value < modulus);
        ModulusInformation {
            modulus: modulus,
            value: value,
        }
    }

    pub fn none() -> Self {
        Self::new(1, 0)
    }

    pub fn combine(&mut self, other: Self, max_modulus: u32) {
        assert!(other.modulus <= self.info_remaining(max_modulus));
        self.value = self.value + self.modulus * other.value;
        self.modulus = std::cmp::min(max_modulus, self.modulus * other.modulus);
        assert!(self.value < self.modulus);
    }

    pub fn info_remaining(&self, max_modulus: u32) -> u32 {
        // We want to find the largest number `result` such that
        // `self.combine(other, max_modulus)` works whenever `other.modulus == result`.
        // `other.value` can be up to `result - 1`, so calling combine could increase our value to
        // up to `self.value + self.modulus * (result - 1)`, which must always be less than
        // `max_modulus`.
        // Therefore, we compute the largest number `result` such that
        // `self.value + self.modulus * (result - 1) < max_modulus`.
        let result = (max_modulus - self.value - 1) / self.modulus + 1;
        assert!(self.value + self.modulus * (result - 1) < max_modulus);
        assert!(self.value + self.modulus * ((result + 1) - 1) >= max_modulus);
        result
    }

    pub fn split(&mut self, modulus: u32) -> Self {
        assert!(self.modulus >= modulus);
        let original_modulus = self.modulus;
        let original_value = self.value;
        let value = self.value % modulus;
        self.value = self.value / modulus;
        // `self.modulus` is the largest number such that
        // `value + (self.modulus - 1) * modulus < original_modulus`.
        // TODO: find an explanation of why this makes everything work out
        self.modulus = (original_modulus - value - 1) / modulus + 1;
        assert!(original_value == value + modulus * self.value);
        Self::new(modulus, value)
    }

    pub fn cast_up(&mut self, modulus: u32) {
        assert!(self.modulus <= modulus);
        self.modulus = modulus;
    }

    // pub fn cast_down(&mut self, modulus: u32) {
    //     assert!(self.modulus >= modulus);
    //     assert!(self.value < modulus);
    //     self.modulus = modulus;
    // }

    pub fn add(&mut self, other: &Self) {
        assert!(self.modulus == other.modulus);
        self.value = (self.value + other.value) % self.modulus;
    }

    pub fn subtract(&mut self, other: &Self) {
        assert!(self.modulus == other.modulus);
        self.value = (self.modulus + self.value - other.value) % self.modulus;
    }
}

pub trait Question {
    // how much info does this question ask for?
    fn info_amount(&self) -> u32;
    // get the answer to this question, given cards
    fn answer(&self, &Cards, &BoardState) -> u32;
    // process the answer to this question, updating card info
    fn acknowledge_answer(
        &self, value: u32, &mut HandInfo<CardPossibilityTable>, &BoardState
    );

    fn answer_info(&self, hand: &Cards, board: &BoardState) -> ModulusInformation {
        ModulusInformation::new(
            self.info_amount(),
            self.answer(hand, board)
        )
    }

    fn acknowledge_answer_info(
        &self,
        answer: ModulusInformation,
        hand_info: &mut HandInfo<CardPossibilityTable>,
        board: &BoardState
    ) {
        assert!(self.info_amount() == answer.modulus);
        self.acknowledge_answer(answer.value, hand_info, board);
    }
}

pub trait PublicInformation: Clone {
    fn get_player_info(&self, &Player) -> HandInfo<CardPossibilityTable>;
    fn set_player_info(&mut self, &Player, HandInfo<CardPossibilityTable>);

    fn new(&BoardState) -> Self;
    fn set_board(&mut self, &BoardState);

    /// If we store more state than just `HandInfo<CardPossibilityTable>`s, update it after `set_player_info` has been called.
    fn update_other_info(&mut self) {
    }

    fn agrees_with(&self, other: Self) -> bool;

    /// By defining `ask_questions`, we decides which `Question`s a player learns the answers to.
    ///
    /// A player "asks" a question by calling the callback. Questions can depend on the answers to
    /// earlier questions: We are given a `&mut HandInfo<CardPossibilityTable>` that we'll have to pass
    /// to that callback; there, it will be modified to reflect the answer to the question. Note that `self`
    /// is not modified and thus reflects the state before any player "asked" any question.
    ///
    /// The product of the `info_amount()`s of all questions we have may not exceed `total_info`.
    /// For convenience, we pass a `&mut u32` to the callback, and it will be updated to the
    /// "remaining" information amount.
    fn ask_questions<Callback>(&self, &Player, &mut HandInfo<CardPossibilityTable>, Callback, total_info: u32)
        where Callback: FnMut(&mut HandInfo<CardPossibilityTable>, &mut u32, Box<Question>);

    fn set_player_infos(&mut self, infos: Vec<(Player, HandInfo<CardPossibilityTable>)>) {
        for (player, new_hand_info) in infos {
            self.set_player_info(&player, new_hand_info);
        }
        self.update_other_info();
    }

    fn get_hat_info_for_player(
        &self, player: &Player, hand_info: &mut HandInfo<CardPossibilityTable>, total_info: u32, view: &OwnedGameView
    ) -> ModulusInformation {
        assert!(player != &view.player);
        let mut answer_info = ModulusInformation::none();
        {
            let callback = |hand_info: &mut HandInfo<CardPossibilityTable>, info_remaining: &mut u32, question: Box<Question>| {
                let new_answer_info = question.answer_info(view.get_hand(player), view.get_board());
                question.acknowledge_answer_info(new_answer_info.clone(), hand_info, view.get_board());
                answer_info.combine(new_answer_info, total_info);
                *info_remaining = answer_info.info_remaining(total_info);
            };
            self.ask_questions(player, hand_info, callback, total_info);
        }
        answer_info.cast_up(total_info);
        answer_info
    }

    fn update_from_hat_info_for_player(
        &self,
        player: &Player,
        hand_info: &mut HandInfo<CardPossibilityTable>,
        board: &BoardState,
        mut info: ModulusInformation,
    ) {
        let total_info = info.modulus;
        {
            let callback = |hand_info: &mut HandInfo<CardPossibilityTable>, info_remaining: &mut u32, question: Box<Question>| {
                let answer_info = info.split(question.info_amount());
                question.acknowledge_answer_info(answer_info, hand_info, board);
                *info_remaining = info.modulus;
            };
            self.ask_questions(player, hand_info, callback, total_info);
        }
        assert!(info.value == 0);
    }

    /// When deciding on a move, if we can choose between `total_info` choices,
    /// `self.get_hat_sum(total_info, view)` tells us which choice to take, and at the same time
    /// mutates `self` to simulate the choice becoming common knowledge.
    fn get_hat_sum(&mut self, total_info: u32, view: &OwnedGameView) -> ModulusInformation {
        if total_info == 1 {
            return ModulusInformation::none();
        }
        let (infos, new_player_hands): (Vec<_>, Vec<_>) = view.get_other_players().iter().map(|player| {
            let mut hand_info = self.get_player_info(player);
            let info = self.get_hat_info_for_player(player, &mut hand_info, total_info, view);
            (info, (player.clone(), hand_info))
        }).unzip();
        self.set_player_infos(new_player_hands);
        infos.into_iter().fold(
            ModulusInformation::new(total_info, 0),
            |mut sum_info, info| {
                sum_info.add(&info);
                sum_info
            }
        )
    }

    /// When updating on a move, if we infer that the player making the move called `get_hat_sum()`
    /// and got the result `info`, we can call `self.update_from_hat_sum(info, view)` to update
    /// from that fact.
    fn update_from_hat_sum(&mut self, mut info: ModulusInformation, view: &OwnedGameView) {
        if info.modulus == 1 {
            return;
        }
        let info_source = view.board.player;
        let (other_infos, mut new_player_hands): (Vec<_>, Vec<_>) = view.get_other_players().into_iter().filter(|player| {
            *player != info_source
        }).map(|player| {
            let mut hand_info = self.get_player_info(&player);
            let player_info = self.get_hat_info_for_player(&player, &mut hand_info, info.modulus, view);
            (player_info, (player.clone(), hand_info))
        }).unzip();
        for other_info in other_infos {
            info.subtract(&other_info);
        }
        let me = view.player;
        if me == info_source {
            assert!(info.value == 0);
        } else {
            let mut my_hand = self.get_player_info(&me);
            self.update_from_hat_info_for_player(&me, &mut my_hand, &view.board, info);
            new_player_hands.push((me, my_hand));
        }
        self.set_player_infos(new_player_hands);
    }

    fn get_private_info(&self, view: &OwnedGameView) -> HandInfo<CardPossibilityTable> {
        let mut info = self.get_player_info(&view.player);
        for card_table in info.iter_mut() {
            for (_, hand) in &view.other_hands {
                for card in hand {
                    card_table.decrement_weight_if_possible(card);
                }
            }
        }
        info
    }

    /// Suppose we as the current player can do some action that others don't know we can, but that
    /// others will recognize once they see it (say we discard a card that we only privately know
    /// to be dead). We can use this to transmit half a bit of information: We get a hat sum; if the
    /// sum is 0, we do that action to transmit this hat information, if the sum is something else,
    /// we don't (and thus don't transmit information since other players won't learn that we could
    /// do the action).
    ///
    /// We will have (roughly) a probability of `1/num_states` of choosing to do the action, and if
    /// we do, we transmit `log(num_states)` bits of  information to each player.
    /// Note that other players need to know how we chose `num_states`.
    // FIXME: talk about optimum!
    // TODO: Randomization here could actually help! (For instance, right now, calling this
    // method twice in a row is kind of useless.) We'd just have to do some careful bookkeeping of
    // our random state.
    fn decide_action_not_known_to_be_possible(&mut self, num_states: u32, view: &OwnedGameView) -> bool {
        let hat_sum = self.clone().get_hat_sum(num_states, view);
        if hat_sum.value == 0 {
            let _ = self.get_hat_sum(num_states, view);
            true
        } else {
            false
        }
    }
    /// If we infer that the player making the move called `decide_action_not_known_to_be_possible()`
    /// and got the result `true`, we call `update_from_action_not_known_to_be_possible`.
    fn update_from_action_not_known_to_be_possible(&mut self, num_states: u32, view: &OwnedGameView) {
        self.update_from_hat_sum(ModulusInformation {
            modulus: num_states,
            value: 0,
        }, view);
    }
}
