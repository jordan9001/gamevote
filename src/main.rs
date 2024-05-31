use std::{
    env,
    time::Duration,
    collections::HashMap,
    sync::{RwLock, Arc},
};
use serenity::{
    async_trait,
    prelude::*,
    futures::StreamExt,
    model::{
        channel::Message,
        gateway::Ready,
        application::interaction::InteractionResponseType,
        application::component::InputTextStyle,
        application::component::ButtonStyle,
        prelude::{component::ActionRowComponent, ChannelId, UserId, User, MessageId},
    },
    collector::{ModalInteractionCollectorBuilder, ComponentInteractionCollectorBuilder},
    builder::CreateComponents,
    utils::{content_safe, ContentSafeOptions},
};
use tallystick::{
    approval::DefaultApprovalTally,
    borda::DefaultBordaTally,
    score::ScoreTally,
};

//TODO:
// 1) test add prompt message
// b> test indicate timeout period (just amount for now)
// then: new ranked choice setup, where each choice is a button that cycles through available positions (and a reset button)

const ID_BUILD_TYPE: &str = "VoteKind";
const ID_BUILD_SUG_BTN: &str = "SugBtn";
const ID_BUILD_SHOWRES_BTN: &str = "ShowResBtn";
const ID_BUILD_VOTEONE_BTN: &str = "OneVoteBtn";
const ID_BUILD_PROMPT_BTN: &str = "PromptBtn";
const ID_BUILD_PROMPT_INPUT: &str = "BuildPromptModal";
const ID_BUILD_PROMPT_INPUT_TXT: &str = "BuildPromptIn";
const ID_BUILD_PING_BTN: &str = "PingBtn";
const ID_BUILD_DUR_BTN: &str = "DurBtn";
const ID_BUILD_CHOICE_BTN: &str = "ValBtn";
const ID_BUILD_SUBMIT: &str = "BuildSubmit";
const ID_BUILD_CANCEL: &str = "BuildCancel";
const ID_BUILD_VAL_INPUT: &str = "BuildValModal";
const ID_BUILD_VAL_INPUT_TXT: &str = "BuildValIn";
const ID_BUILD_DUR_INPUT: &str = "BuildDurModal";
const ID_BUILD_DUR_INPUT_TXT: &str = "BuildDurIn";
const ID_SUG_VAL_BTN: &str = "SugVBtn";
const ID_SUG_VAL_INPUT: &str = "SugVBtn";
const ID_SUG_VAL_INPUT_TXT: &str = "SugVBtn";
const ID_SUG_SUB_BTN: &str = "SugSubBtn";
const ID_VOTE_VAL_PREFIX: &str = "VoteVal";
const ID_VOTE_VAL_INPUT: &str = "ValInModal";
const ID_VOTE_VAL_INPUT_PREFIX: &str = "ValIn";
const ID_VOTE_BTN: &str = "MainVoteBtn";
const ID_VOTE_LEFT: &str = "VoteLeft";
const ID_VOTE_RIGHT: &str = "VoteRight";
const ID_VOTE_SUBMIT: &str = "VoteSubmit";

const VOTE_DM_CONT: &str = "Create a new Vote:";

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60*90);
const VOTE_DM_TIMEOUT: Duration = Duration::from_secs(60*60*1);
const MAX_DUR_HR: f64 = 24.0*6.0;
const MIN_DUR_HR: f64 = 0.01;
const PERPAGE: usize = 4;

#[derive(PartialEq, Eq, Debug, Clone, Copy)]
struct VoteType(u32);
const VOTE_APPROVAL: VoteType   = VoteType(1 << 0);
const VOTE_SCORE: VoteType      = VoteType(1 << 1);
const VOTE_LSCORE: VoteType     = VoteType(1 << 2);
const VOTE_BORDA: VoteType      = VoteType(1 << 3);

impl ToString for VoteType {
    fn to_string(&self) -> String {
        match *self {
            VOTE_APPROVAL => "Approval".into(),
            VOTE_SCORE => "Score".into(),
            VOTE_LSCORE => "Limited Score".into(),
            VOTE_BORDA => "Borda".into(),
            _ => panic!("Unknown vote type! {:?}", self),
        }
    }
}

impl VoteType {
    fn from_string(s: &String) -> Self {
        match &s[..] {
            "Approval" => VOTE_APPROVAL,
            "Score" => VOTE_SCORE,
            "Limited Score" => VOTE_LSCORE,
            "Borda" => VOTE_BORDA,
            _ => panic!("Tried to get a vote type from an unknown string! {}", s),
        }
    }

    fn value_name(&self) -> String {
        match *self {
            VOTE_APPROVAL => "choice".into(),
            VOTE_SCORE => "score (-10.0 to 10.0)".into(),
            VOTE_LSCORE => "score where sum(abs(scores)) <= 10.0".into(),
            VOTE_BORDA => "rank (1 is 1st choice, 2 second, ...)".into(),
            _ => panic!("Tried to get value name for unknown vote type! {:?}", self),
        }
    }

    fn is_bad_value(&self, v: f32, vals: &Vec<String>) -> bool {
        match *self {
            VOTE_APPROVAL => false,
            VOTE_SCORE | VOTE_LSCORE => v < -10.0 || v > 10.0,
            VOTE_BORDA => v.fract() != 0.0 || v <= 0.0 || v > (vals.len() as f32),
            _ => panic!("Tried to test value for unknown vote type! {:?}", self),
        }
    }

    fn get_all() -> Vec<Self> {
        vec![VOTE_APPROVAL, VOTE_SCORE, VOTE_LSCORE, VOTE_BORDA]
    }
}


#[derive(Debug)]
struct VoteInfo {
    kind: VoteType,
    prompt: String,
    take_sugs: bool,
    show_at_timeout: bool,
    vote_once: bool,
    show_timeout: bool, // TODO
    allow_early_stop: bool, // TODO
    ping_chan: u8,
    timeout: Duration,
    vals: Vec<String>,
}

impl VoteInfo {
    fn new() -> Self {
        VoteInfo {
            kind: VOTE_APPROVAL,
            prompt: "".into(),
            take_sugs: false,
            show_at_timeout: true,
            vote_once: false,
            show_timeout: true,
            allow_early_stop: true,
            ping_chan: 0,
            timeout: DEFAULT_TIMEOUT,
            vals: Vec::new(),
        }
    }

    fn submittable(&self) -> bool {
        self.vals.len() > 1 || self.take_sugs
    }

    fn get_timeout_str(&self, suffix: &str) -> String {
        let fsec = self.timeout.as_secs_f64() / (60.0 * 60.0);
        if fsec == 0.0 {
            "".into()
        } else if fsec.fract() == 0.0 {
            format!("{}{}", fsec, suffix)
        } else {
            format!("{:.1}{}", fsec, suffix)
        }
    }

    fn get_ping(&self) -> String {
        match self.ping_chan {
            0 => "".into(),
            1 => "@here ".into(),
            2 => "@everyone ".into(),
            _ => panic!("Unknown ping_chan value"),
        }
    }
}

enum CastVotes {
    Select(Vec<usize>), // one or more choices, used for normal or approval voting
    Score(HashMap<usize, f32>), // choices associated with a value
    Rank(HashMap<usize, usize>), // rank voting
}

impl CastVotes {
    fn new(vt: VoteType) -> Self {
        match vt {
            VOTE_APPROVAL => CastVotes::Select(Vec::new()),
            VOTE_SCORE => CastVotes::Score(HashMap::new()),
            VOTE_LSCORE => CastVotes::Score(HashMap::new()),
            VOTE_BORDA => CastVotes::Rank(HashMap::new()),
            _ => panic!("Tried to create CastVotes with unknown vote type"),
        }
    }

    fn get_vote_vec(&self) -> Vec<usize> {
        // if this is a score, then order them from lowest to highest as a ranking
        // otherwise just return the vec
        match self {
            CastVotes::Select(v) => {
                v.to_vec()
            },
            CastVotes::Score(_m) => {
                panic!("Tried to get a ordered vec, but have a Score type");
            },
            CastVotes::Rank(m) => {
                let mut vt: Vec<(usize, usize)> = Vec::new();
                for (u, f) in m {
                    vt.push((*u,*f));
                }
                vt.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
                vt.iter().map(|x| x.0).collect()
            },
        }
    }

    fn get_vote_weight_vec(&self) -> Vec<(usize, f32)> {
        // if this is a select, then something went wrong
        match self {
            CastVotes::Select(_) => {
                panic!("Tried to get weighted vec but have a select");
            },
            CastVotes::Score(m) => {
                let mut v: Vec<(usize, f32)> = Vec::new();
                for (u, f) in m {
                    v.push((*u,*f));
                }
                v
            }
            CastVotes::Rank(_) => {
                panic!("Tried to get weighted vec but have a rank");
            },
        }
    }

    fn are_valid(&self, vt: VoteType, size: usize) -> bool {
        match vt {
            VOTE_APPROVAL => true,
            VOTE_SCORE => true,
            VOTE_LSCORE => {
                // check the sum(abs(scores))
                let mut abssum: f32 = 0.0f32;

                if let CastVotes::Score(hm) = self {
                    for (_, score) in hm.iter() {
                        abssum += score.abs();
                    }
                } else {
                    panic!("Tried to check validity of a LSCORE with no score backing");
                }

                abssum < 10.0001f32
            }
            VOTE_BORDA => {
                // check each key has a non-zero rating (default is size-1)
                // not too worried about the ratings, just let the sorting sort it
                if let CastVotes::Rank(m) = self {
                    for i in 0..size {
                        if let Some(v) = m.get(&i) {
                            if *v < 1 {
                                return false;
                            }
                        } else {
                            return false;
                        }

                    }
                } else {
                    panic!("Tried to check validity of a BORDA with out a rank backing");
                }
                true
            },
            _ => panic!("Tried to check votes with unknown vote type"),
        }
    }

    fn get_ballot(&self, size: usize) -> Self {
        match self {
            CastVotes::Select(v) => {
                let newv = v.to_vec();
                CastVotes::Select(newv)
            },
            CastVotes::Score(m) => {
                let mut newm = m.clone();
                // add in defaults
                for i in 0..size {
                    if !newm.contains_key(&i) {
                        newm.insert(i, 0.0);
                    }
                }
                CastVotes::Score(newm)
            },
            CastVotes::Rank(m) => {
                let mut newm = m.clone();
                // add in defaults
                for i in 0..size {
                    if !newm.contains_key(&i) {
                        newm.insert(i, size-1);
                    }
                }
                CastVotes::Rank(newm)
            }
        }
    }
}

struct UserVote {
    votes: CastVotes,
    votemsg: MessageId,
    page: usize,
}

struct Vote {
    kind: VoteType,
    uservotes: HashMap<UserId,UserVote>,
    submittedvotes: HashMap<UserId,CastVotes>,
}


macro_rules! tally_str {
    ($tally:expr, $vals:expr, $votetype:expr, $num_voters:expr, $extra:expr) => {
    {
        let mut result: String = format!("{} Vote Results (with {} voters):\nWinner:\n", $votetype.to_string(), $num_voters);
        let winners = $tally.winners().all();
        for w in winners {
            result.push_str(&format!("{}\n", $vals[w]));
        }
        result.push_str("\nTotals:\n");
        for (w, c) in $tally.totals() {
            result.push_str(&format!("{}: {}\n", c, $vals[w]));
        }
        result.push_str($extra);
        result
    }
    };
}

impl Vote {
    fn new(vt: VoteType) -> Self {
        Vote {
            kind: vt,
            uservotes: HashMap::new(),
            submittedvotes: HashMap::new(),
        }
    }

    fn get_results(&self, vals: &Vec<String>, extra: &str) -> String {
        let mut num_voters = 0;
        match self.kind {
            VOTE_APPROVAL => {
                let mut tally = DefaultApprovalTally::new(1);

                for (_, cv) in &self.submittedvotes {
                    tally.add(cv.get_vote_vec());
                    num_voters += 1;
                }

                tally_str!(tally, vals, self.kind, num_voters, extra)
            },
            VOTE_SCORE | VOTE_LSCORE => {
                let mut tally = ScoreTally::<usize, f32>::new(1);

                for (_, cv) in &self.submittedvotes {
                    tally.add(cv.get_vote_weight_vec());
                    num_voters += 1;
                }

                tally_str!(tally, vals, self.kind, num_voters, extra)
            },
            VOTE_BORDA => {
                let mut tally = DefaultBordaTally::new(1, tallystick::borda::Variant::Borda);

                for (_, cv) in &self.submittedvotes {
                    tally.add(cv.get_vote_vec()).unwrap();
                    num_voters += 1;
                }

                tally_str!(tally, vals, self.kind, num_voters, extra)
            },
            _ => panic!("Tried to get results with unknown vote type"),
        }
    }
}

// a macro because the builder for creating and editing have the same functions, but different types
// maybe serenity should put those in a trait
macro_rules! setup_base_message {
    ($prompt:expr, $timestr:expr, $m:expr, $num_votes:expr, $vtype:expr, $ping:expr) => {
        //TODO show timeout time nicer
        $m
            .content(format!("{}{}{}{} Vote: {} Votes so far\n", $prompt, $ping, $timestr, $vtype, $num_votes))
            .components(|c| {
                c.create_action_row(|r| {
                    r.create_button(|btn| {
                        btn.custom_id(ID_VOTE_BTN)
                            .style(ButtonStyle::Primary)
                            .label("Vote!")
                    })
                })
            })
    };
}

fn create_user_message<'a, 'b, 'c>(mut c: &'b mut CreateComponents, vals: &'c Vec<String>, page: usize, vote: &'c Vote, uid: UserId) -> &'b mut CreateComponents {
    let i = page * PERPAGE;
    for j in 0..PERPAGE {
        let vali = i + j;
        if vali >= vals.len() {
            break;
        }
        let item_note = if let Some(uv) = vote.uservotes.get(&uid) {
            match &uv.votes {
                CastVotes::Select(v) => {
                    String::from(if v.contains(&vali) {" ✅"} else {""})
                },
                CastVotes::Score(m) => {
                    let s = if let Some(score) = m.get(&vali) {
                        *score
                    } else {
                        0.0
                    };
                    format!(": {}", s)
                },
                CastVotes::Rank(m) => {
                    let s = if let Some(score) = m.get(&vali) {
                        *score
                    } else {
                        vals.len()
                    };
                    format!(": Rank {}", s)
                }
            }
        } else {
            String::from("")
        };

        c = c.create_action_row(|r| {
            r.create_button(|btn| {
                btn.custom_id(format!("{}{}", ID_VOTE_VAL_PREFIX, vali))
                    .style(ButtonStyle::Secondary)
                    .label(format!("{}{}", vals[vali], item_note))
            })
        });
    }
    // add a row for the movement and submit buttons
    let nonav = vals.len() <= PERPAGE && page == 0;
    c.create_action_row(|mut r| {
        if !nonav {
            r = r
                .create_button(|btn| {
                    btn.custom_id(ID_VOTE_LEFT)
                        .style(ButtonStyle::Secondary)
                        .label("<")
                })
                .create_button(|btn| {
                    btn.custom_id(ID_VOTE_RIGHT)
                        .style(ButtonStyle::Secondary)
                        .label(">")
                });
        }
        r.create_button(|btn| {
            btn.custom_id(ID_VOTE_SUBMIT)
                .style(ButtonStyle::Primary)
                .label("Submit")
        })
    })
}

// a macro because the different interaction types
macro_rules! user_vote_message {
    ($interaction:expr, $uid:expr, $extra:expr, $vote:expr, $ctx:expr, $num_pages:expr, $vals:expr, $first:expr, $vote_once:expr) => {
        // edit or create the ephemeral
        let irkind: InteractionResponseType = if $first {
            InteractionResponseType::ChannelMessageWithSource
        } else {
            InteractionResponseType::UpdateMessage
        };

        let mut can_vote = true;

        let disppage = {
            let rvote = $vote.read().unwrap();

            if $vote_once {
                can_vote = !rvote.submittedvotes.contains_key(&$uid);
            }

            if let Some(uv) = rvote.uservotes.get(&$uid) {
                uv.page
            } else {
                panic!("Tried to get user page for user not in vote structure yet");
            }
        };

        if can_vote {
            $interaction.create_interaction_response($ctx, |resp| {
                resp.kind(irkind).interaction_response_data(|d| {
                    d
                        .content(format!("Page {}/{}{}", disppage+1, $num_pages, $extra))
                        .components(|c| {
                            let rvote = $vote.read().unwrap();
                            create_user_message(c, &$vals, disppage, &rvote, $uid)
                        })
                        .ephemeral(true)
                })
            }).await.unwrap();
        } else {
            // just give them a button to go forward
            $interaction.create_interaction_response($ctx, |resp| {
                resp.kind(irkind).interaction_response_data(|d| {
                    d
                        .content(format!("Vote Submitted{}", $extra))
                        .components(|c| {
                            c.create_action_row(|r| {
                                r.create_button(|btn| {
                                    btn.custom_id(ID_VOTE_SUBMIT)
                                        .style(ButtonStyle::Primary)
                                        .label("View Results")
                                })
                            })
                        })
                        .ephemeral(true)
                })
            }).await.unwrap();
        }
    };
}

async fn start_vote(ctx: &Context, cid: ChannelId, vi: VoteInfo) {
    let pingstr = vi.get_ping();
    let timestr = vi.get_timeout_str(" hr ");
    let VoteInfo{kind: votetype, mut vals, timeout, show_at_timeout, vote_once, .. } = vi;

    let vote = Arc::new(RwLock::new(Vote::new(votetype)));
    let num_pages = ((vals.len() -1) / PERPAGE) + 1;

    // actually let's try just having a "vote" button, so we can edit the ephemeral button to match each user
    let basemsg = cid.send_message(ctx, |m| {
        setup_base_message!(vi.prompt, timestr, m, 0, votetype.to_string(), pingstr)
    }).await.unwrap();

    // first let's keep each game name under 33 char
    for v in &mut vals {
        v.truncate(33)
    }

    // send a ephemeral message (or multiple) to the channel for everyone, with the voting options
    // only 5 rows per message, so we have <> btns
    // we need to 

    // let's build multiple collectors and select on them

    let mut mainbtn_col = basemsg.await_component_interactions(ctx)
        .timeout(timeout)
        .build();
    
    // collector for the left, right, submit buttons
    let nav_vote = vote.clone();
    let mut nav_col = ComponentInteractionCollectorBuilder::new(ctx)
        .timeout(timeout)
        .channel_id(cid)
        .filter(move |i| {
            // filter by our ephemeral message id
            let mid = i.message.id;
            let uid = i.user.id;

            {
                let rvote = nav_vote.read().unwrap();
                if let Some(uv) = rvote.uservotes.get(&uid) {
                    if mid == uv.votemsg {
                        return true;
                    }
                }
            }
            false
        })
        .build();

    // collector for the modals from some votes
    let mod_vote = vote.clone();
    let mut mod_col = ModalInteractionCollectorBuilder::new(ctx)
        .timeout(timeout)
        .channel_id(cid)
        .filter(move |i| {
            // filter on the ephemeral message for this vote
            let uid = i.user.id;

            if let Some(msg) = &i.message {
                let mid = msg.id;
                let rvote = mod_vote.read().unwrap();
                if let Some(uv) = rvote.uservotes.get(&uid) {
                    if mid == uv.votemsg {
                        return true;
                    }
                }
            }
            false
        })
        .build();

    // This is our select loop where we wait on any interactions relevant to this vote
    loop {
        tokio::select! {
            Some(interaction) = mainbtn_col.next() => {
                
                let uid = interaction.user.id;

                // lookup / init user

                {
                    let mut wvote = vote.write().unwrap();

                    if !wvote.uservotes.contains_key(&uid) {
                        wvote.uservotes.insert(uid, UserVote{
                            votes: CastVotes::new(votetype),
                            votemsg: MessageId(0),
                            page: 0,
                        });
                    }
                }

                // this button brings up a new ephemeral message for voting
                user_vote_message!(interaction, uid, "", vote, ctx, num_pages, vals, true, vote_once);

                // get the response info
                let respmsg = interaction.get_interaction_response(ctx).await.unwrap();

                {
                    let mut wvote = vote.write().unwrap();

                    if let Some(uv) = wvote.uservotes.get_mut(&uid) {
                        // delete any old ephemeral message? How?
                        // set the message id from the new ephemeral message
                        uv.votemsg = respmsg.id
                    } else {
                        panic!("Tried to set initial ephemeral message, but didn't have a entry for the user");
                    }
                }
            },
            Some(interaction) = nav_col.next() => {
                let uid = interaction.user.id;

                match &interaction.data.custom_id[..] {
                    lr @ (ID_VOTE_LEFT | ID_VOTE_RIGHT) => {
                        let dir = lr == ID_VOTE_LEFT;
                        {
                            let mut wvote = vote.write().unwrap();

                            if let Some(uv) = wvote.uservotes.get_mut(&uid) {
                                // edit their message to the next page to the left
                                let mut page = uv.page;
                                if !dir {
                                    page += 1;
                                    if page >= num_pages {
                                        page = 0;
                                    }
                                } else {
                                    if page == 0 {
                                        page = num_pages - 1;
                                    } else {
                                        page -= 1;
                                    }
                                }

                                uv.page = page;

                            } else {
                                panic!("Somehow got a nav interaction without an entry in the vote map?")
                            }
                        }
                        user_vote_message!(interaction, uid, "", vote, ctx, num_pages, vals, false, vote_once);
                    },
                    ID_VOTE_SUBMIT => {
                        let mut isfirst = false;
                        let mut dosubmit = true;
                        let mut showresults = false;

                        // if vote_once, we need to check if they have already voted, and if so just show the results
                        if vote_once {
                            {
                                let rvote = vote.read().unwrap();
                                if !rvote.submittedvotes.contains_key(&uid) {
                                    isfirst = true;
                                }
                            };

                            if !isfirst {
                                showresults = true;
                                dosubmit = false;
                            }
                        }

                        if dosubmit {
                            // submit the vote for this user, if we can
                            // first check that it is a valid submission, and let them know if it is not
                            let valid_submission;

                            let mut subcount = 0;
                            {
                                let mut wvote = vote.write().unwrap();

                                if let Some(uv) = wvote.uservotes.get_mut(&uid) {
                                    valid_submission = uv.votes.are_valid(votetype, vals.len());

                                    if valid_submission {
                                        let ballot = uv.votes.get_ballot(vals.len());
                                        if wvote.submittedvotes.insert(uid, ballot).is_none() {
                                            subcount = wvote.submittedvotes.len();
                                            isfirst = true;
                                        }
                                    }
                                } else {
                                    panic!("No user but got submit");
                                }
                            }

                            if !valid_submission {
                                // return an error to the user
                                let errresp = format!("\nError: invalid values for a {} vote, please fix your vote.\nEach should be a {}", votetype.to_string(), votetype.value_name());
                                user_vote_message!(interaction, uid, errresp, vote, ctx, num_pages, vals, false, vote_once);
                            } else {
                                // update the count
                                if isfirst {
                                    cid.edit_message(ctx, basemsg.id, |e| {
                                        setup_base_message!(vi.prompt, timestr, e, subcount, votetype.to_string(), pingstr)
                                    }).await.unwrap();
                                }
                                showresults = true;

                            }
                        }

                        if showresults {
                            // calculate the vote result
                            let resultsmsg;
                            {
                                let rvote = vote.read().unwrap();

                                resultsmsg = rvote.get_results(&vals, "\n Submit again to get fresh results");
                            }

                            // show them the vote results message
                            // it would be cool to update everyone's messages
                            // but we can't do that to an ephemeral message without an interaction to respond to

                            interaction.create_interaction_response(ctx, |resp| {
                                resp.kind(InteractionResponseType::ChannelMessageWithSource).interaction_response_data(|d| {
                                    d
                                        .content(resultsmsg)
                                        .ephemeral(true)
                                })
                            }).await.unwrap();
                        }
                    },
                    value_id => {
                        // find which value the vote is for
                        if !value_id.starts_with(ID_VOTE_VAL_PREFIX) {
                            panic!("Unknown component value from voting message {}", value_id);
                        }

                        let num = value_id[ID_VOTE_VAL_PREFIX.len()..].parse::<usize>().unwrap();
                        let mut current_score_f: f32 = 0.0;
                        let mut refresh_msg = true;

                        println!("Vote for value {} ({})", num, vals[num]);

                        {
                            let mut wvote = vote.write().unwrap();

                            if let Some(uv) = wvote.uservotes.get_mut(&uid) {
                                // depending on the vote type, pop a modal to ask for more info
                                // otherwise just toggle this one in the vote list
                                match &mut uv.votes {
                                    CastVotes::Select(v) => {
                                        if v.contains(&num) {
                                            v.retain(|&x| x != num);
                                        } else {
                                            v.push(num);
                                        }
                                    },
                                    CastVotes::Score(m) => {
                                        // show a modal to collect a float number from them
                                        if let Some(score) = m.get(&num) {
                                            current_score_f = *score;
                                        }
                                        // we need to drop the lock before we await and create the modal
                                        // don't update the message yet, we will do that after the modal
                                        refresh_msg = false;
                                    },
                                    CastVotes::Rank(m) => {
                                        // rank++
                                        let rank = match m.get(&num) {
                                            Some(score) => *score,
                                            _ => vals.len(),
                                        };

                                        let mut rank = rank + 1;
                                        if rank > vals.len() {
                                            rank = 1;
                                        }
                                        
                                        m.insert(num, rank);
                                        // no modal, just refresh the message
                                    },
                                }
                            } else {
                                panic!("Somehow got a vote interaction without an entry in the vote map?")
                            }
                        }

                        if !refresh_msg {
                            interaction.create_interaction_response(ctx, |resp| {
                                resp.kind(InteractionResponseType::Modal).interaction_response_data(|d| {
                                    let title = format!("Vote for {}", vals[num]);
                                    d.custom_id(ID_VOTE_VAL_INPUT)
                                        .title(title)
                                        .components(|c| {
                                            c.create_action_row(|r| {
                                                // create text input for adding the options
                                                r.create_input_text(|t| {
                                                    t.custom_id(format!("{}{}", ID_VOTE_VAL_INPUT_PREFIX, num))
                                                        .style(InputTextStyle::Short)
                                                        .label(votetype.value_name())
                                                        .value(current_score_f.to_string())
                                                        .min_length(1)
                                                        .max_length(5)
                                                        .required(true)
                                                })
                                            })
                                        })
                                })
                            }).await.unwrap();
                        } else {
                            user_vote_message!(interaction, uid, "", vote, ctx, num_pages, vals, false, vote_once);
                        }
                    },
                }
            },
            Some(interaction) = mod_col.next() => {
                println!("Got Modal Vote Interaction");
                if let ActionRowComponent::InputText(it) = &interaction.data.components[0].components[0] {
                    let uid = interaction.user.id;

                    // parse custom id to get vote index
                    if !it.custom_id.starts_with(ID_VOTE_VAL_INPUT_PREFIX) {
                        panic!("Unknown component value from voting message {}", it.custom_id);
                    }

                    let num = it.custom_id[ID_VOTE_VAL_INPUT_PREFIX.len()..].parse::<usize>().unwrap();

                    let badvalue;
                    // parse value to get f32 value
                    let score: f32 = match it.value.parse::<f32>() {
                        Ok(s) => {
                            badvalue = votetype.is_bad_value(s, &vals);
                            if badvalue {
                                0.0
                            } else {
                                s
                            }
                        },
                        Err(_) => {
                            badvalue = true;
                            0.0
                        },
                    };

                    let errresp = if badvalue {
                        println!("Changing bad value {:?} to 0.0", it.value);
                        format!("\nError: Bad Value")
                    } else {
                        String::from("")
                    };

                    {
                        let mut wvote = vote.write().unwrap();

                        if let Some(uv) = wvote.uservotes.get_mut(&uid) {
                            match &mut uv.votes {
                                CastVotes::Select(_v) => {
                                    panic!("Got modal response for a select vote?");
                                },
                                CastVotes::Score(m) => {
                                    m.insert(num, score);
                                },
                                CastVotes::Rank(_m) => {
                                    panic!("Got modal response for a rank vote?");
                                },
                            }
                        } else {
                            panic!("Somehow got a modal interaction without an entry in the vote map?")
                        }
                    }

                    // edit the ephemeral
                    user_vote_message!(interaction, uid, errresp, vote, ctx, num_pages, vals, false, vote_once);
                } else {
                    panic!("Modal response didn't have input text?");
                }
            }
            else => {
                //TODO could be from a broken connection as well or something? Get out while you can
                println!("Ending collection for vote! Timed out");
                break;
            }
        };
    } // end select loop

    // update the main message to indicate the vote is over, could display final results too, depending on settings
    cid.edit_message(ctx, basemsg.id, |e| {
        e.content(
            if show_at_timeout {
                let rvote = vote.read().unwrap();
                rvote.get_results(&vals, "\nThanks!")
            } else {
                format!("Vote Finished")
            }
        ).components(|c| c)
    }).await.unwrap();

}

fn create_sug_comp<'a, 'b>(mut c: &'a mut CreateComponents) -> &'a mut CreateComponents {
    // vote suggestion modal
    c = c.create_action_row(|r| {
        r.create_button(|b| {
            b
                .custom_id(ID_SUG_VAL_BTN)
                .style(ButtonStyle::Primary)
                .label("Add Suggestion")
        })
    });

    // finish suggestions, start vote
    c = c.create_action_row(|r| {
        r.create_button(|b| {
            b
                .custom_id(ID_SUG_SUB_BTN)
                .style(ButtonStyle::Secondary)
                .label("Start Vote")
        })
    });

    c
}

macro_rules! setup_sug_message {
    ($m:expr, $vi:expr) => {
        {
            //TODO show timeout time nicer when no timeout
            let mut sug_msg = format!("{}{}Submit suggestions for the vote:\n{}\nSuggestions so far:\n", $vi.get_ping(), $vi.get_timeout_str(" hr "), $vi.prompt);

            for c in &$vi.vals {
                sug_msg.push_str(&c);
                sug_msg.push_str("\n");
            }

            $m
                .content(sug_msg)
                .components(create_sug_comp)
        }
    };
}

async fn handle_suggestion_phase(ctx: &Context, author: &User, cid: ChannelId, mut vi: VoteInfo) {
    // create the message in channel inviting choices

    // the vote creator can edit them all, removing suggestions, and can submit
    // if others try to submit, they get an ephemeral msg saying "only _ can"

    let mut msg = cid.send_message(&ctx, |m| {
        setup_sug_message!(m, vi)
    }).await.unwrap();

    // create the collectors
    let mut m_col = msg.await_component_interactions(&ctx)
        .timeout(vi.timeout)
        .build();

    // modal interactions collector
    let mut mod_col = ModalInteractionCollectorBuilder::new(&ctx)
        .timeout(vi.timeout)
        .message_id(msg.id)
        .build();

    // handle events
    let mut do_vote = false;
    loop {
        tokio::select! {
            Some(interaction) = m_col.next() => {
                let is_author = interaction.user.id == author.id;

                match &interaction.data.custom_id[..] {
                    ID_SUG_VAL_BTN => {
                        // author can edit everything, others just get a one line thing
                        interaction.create_interaction_response(&ctx, |resp| {
                            resp.kind(InteractionResponseType::Modal).interaction_response_data(|d| {
                                d
                                    .custom_id(ID_SUG_VAL_INPUT)
                                    .title(
                                        if is_author {
                                            "Edit Choices"
                                        } else {
                                            "Add a Choice"
                                        }
                                    )
                                    .components(|c| {
                                        c.create_action_row(|r| {
                                            r.create_input_text(|mut t| {
                                                t = t
                                                    .custom_id(ID_SUG_VAL_INPUT_TXT)
                                                    .required(true);
                                                if !is_author {
                                                    t = t
                                                        .style(InputTextStyle::Short)
                                                        .label("Choice")
                                                        .min_length(1)
                                                        .max_length(60);
                                                } else {
                                                    t = t
                                                        .style(InputTextStyle::Paragraph)
                                                        .label("Choices (one per line)")
                                                        .min_length(1)
                                                        .max_length(1200);

                                                        if vi.vals.len() > 0 {
                                                            t = t.value(vi.vals.join("\n"));
                                                        }
                                                }

                                                t
                                            })
                                        })
                                    })
                            })
                        }).await.unwrap();
                    },
                    ID_SUG_SUB_BTN => {
                        // only the author can hit this
                        if !is_author {
                            interaction.create_interaction_response(&ctx, |resp| {
                                resp.kind(InteractionResponseType::ChannelMessageWithSource).interaction_response_data(|d| {
                                    d
                                        .content(format!("Sorry, only {} can start the vote", author.name))
                                        .ephemeral(true)
                                })
                            }).await.unwrap();
                        } else {
                            interaction.create_interaction_response(&ctx, |resp| {
                                resp.kind(InteractionResponseType::UpdateMessage).interaction_response_data(|d| {
                                    d
                                        .content("Starting Vote...")
                                        .components(|c| c)
                                })
                            }).await.unwrap();

                            // actually move on now
                            do_vote = true;
                            break;
                        }
                    }
                    _ => {
                        panic!("Got unexpected button id on the sug msg");
                    },
                }
            },
            Some(interaction) = mod_col.next() => {
                let is_author = interaction.user.id == author.id;

                match &interaction.data.custom_id[..] {
                    ID_SUG_VAL_INPUT => {
                        if let ActionRowComponent::InputText(it) = &interaction.data.components[0].components[0] {
                            let v = content_safe(&ctx, &it.value, &ContentSafeOptions::default(), &[]);

                            // if this is from the author, we need to replace everything
                            // otherwise just add them on if they are unique

                            let newvals = v.split('\n').map(|x| String::from(x.trim())).collect();

                            if is_author {
                                vi.vals = newvals;
                            } else {
                                for val in newvals {
                                    if !vi.vals.contains(&val) {
                                        vi.vals.push(val);
                                    }
                                }
                            }

                        } else {
                            panic!("No input found on sug modal");
                        }
                    },
                    _ => {
                        panic!("Got unexpected modal id on the sug msg");
                    }
                }

                // update the dm
                interaction.create_interaction_response(&ctx, |resp| {
                    resp.kind(InteractionResponseType::UpdateMessage).interaction_response_data(|d| {
                        setup_sug_message!(d, vi)
                    })
                }).await.unwrap();
            }
            else => {
                println!("Ending collection for sug msg! Timed out");
                // update the msg to say so

                if vi.vals.len() > 1 {
                    msg.edit(&ctx, |e| {
                        e.content("Time Up! Starting Vote...").components(|c| c)
                    }).await.unwrap();

                    do_vote = true;

                } else {
                    msg.edit(&ctx, |e| {
                        e.content("Time Up! Insufficient options to start vote").components(|c| c)
                    }).await.unwrap();
                }
                break;
            }
        }
    } // end select loop

    // drop the collectors first, for cleanliness
    drop(m_col);
    drop(mod_col);

    if do_vote {
        start_vote(ctx, cid, vi).await;
    }
}

fn create_dm_vote_comp<'a, 'b>(mut c: &'a mut CreateComponents, vi: &'b VoteInfo) -> &'a mut CreateComponents {
    // vote type selection and prompt
    c = c.create_action_row(|mut r| {
        r = r.create_select_menu(|u| {
            u
                .custom_id(ID_BUILD_TYPE)
                .min_values(1)
                .max_values(1)
                .options(|mut o| {
                    for vtype in VoteType::get_all() {
                        o = o.create_option(|mut p| {
                            p = p.label(vtype).value(vtype);
                            // if this is the selected, set it
                            if vi.kind == vtype {
                                p = p.default_selection(true);
                            }
                            p
                        });
                    }
                    o
                })
        });
        r
    });
    // options
    c = c.create_action_row(|mut r| {
        // Prompt
        r = r.create_button(|b| {
            b
                .custom_id(ID_BUILD_PROMPT_BTN)
                .style(ButtonStyle::Secondary)
                .label(
                    if vi.prompt.len() == 0 {
                        "No Prompt"
                    } else {
                        "Prompt Set"
                    }
                )
        });
        // suggestion phase
        r = r.create_button(|b| {
            b
                .custom_id(ID_BUILD_SUG_BTN)
                .style(ButtonStyle::Secondary)
                .label(format!("Take Vote Choice Suggestions = {}",
                    if vi.take_sugs {
                        "Yes"
                    } else {
                        "No"
                    }
                ))
        });

        r = r.create_button(|b| {
            b
                .custom_id(ID_BUILD_SHOWRES_BTN)
                .style(ButtonStyle::Secondary)
                .label(format!("Show Results at Timeout = {}",
                    if vi.show_at_timeout {
                        "Yes"
                    } else {
                        "No"
                    }
                ))
        });

        r = r.create_button(|b| {
            b
                .custom_id(ID_BUILD_VOTEONE_BTN)
                .style(ButtonStyle::Secondary)
                .label(format!("Can Resubmit Vote = {}",
                    if !vi.vote_once {
                        "Yes"
                    } else {
                        "No"
                    }
                ))
        });

        r
    });

    c = c.create_action_row(|mut r| {
        // ping options
        r = r.create_button(|b| {
            b
                .custom_id(ID_BUILD_PING_BTN)
                .style(ButtonStyle::Secondary)
                .label(format!("Ping Channel = {}",
                    match vi.ping_chan {
                        0 => "No",
                        1 => "@here",
                        2 => "@everyone",
                        _ => panic!("ping_chan invalid value"),
                    }
                ))
        });

        // timeout
        r = r.create_button(|b| {
            b
                .custom_id(ID_BUILD_DUR_BTN)
                .style(ButtonStyle::Secondary)
                .label(format!("Vote Timeout = {}",
                    vi.get_timeout_str(" hr")
                ))
        });

        r
    });
    // add/edit choices
    c = c.create_action_row(|r| {
        r.create_button(|b|{
            b
                .custom_id(ID_BUILD_CHOICE_BTN)
                .style(ButtonStyle::Primary)
                .label(
                    if vi.vals.len() == 0 {
                        String::from("Add Vote Choices")
                    } else {
                        format!("Edit Choices ({} choices)", vi.vals.len())
                    }
                )
        })
    });
    // submit, cancel
    c = c.create_action_row(|mut r| {
        r = r.create_button(|b| {
            b
                .custom_id(ID_BUILD_SUBMIT)
                .style(ButtonStyle::Success)
                .label("Start")
                .disabled(!vi.submittable())
        });
        r = r.create_button(|b| {
            b
                .custom_id(ID_BUILD_CANCEL)
                .style(ButtonStyle::Danger)
                .label("Cancel")
        });

        r
    });
    c
}

async fn handle_dm_vote(ctx: Context, msg: Message) {
    let mut vi = VoteInfo::new();

    // create initial dm to the person creating the vote
    // this will get edited as options are changed
    let mut dm: Message = msg.author.direct_message(&ctx, |m| {
        m.content(VOTE_DM_CONT).components(|c| {
            create_dm_vote_comp(c, &vi)
        })
    }).await.unwrap();


    // create collectors for the interaction with the DM and it's modals

    let mut dm_col = dm.await_component_interactions(&ctx)
        .timeout(VOTE_DM_TIMEOUT)
        .build();

    // modal interactions collector
    let mut mod_col = ModalInteractionCollectorBuilder::new(&ctx)
        .timeout(VOTE_DM_TIMEOUT)
        .message_id(dm.id)
        .build();

    // select on the interactions for a given time
    let mut do_vote = false;
    loop {
        tokio::select! {
            Some(interaction) = dm_col.next() => {
                let mut update_dm = true;
                match &interaction.data.custom_id[..] {
                    ID_BUILD_TYPE => {
                        // collect the chosen type
                        vi.kind = VoteType::from_string(&interaction.data.values[0]);
                    },
                    ID_BUILD_SUG_BTN => {
                        vi.take_sugs = !vi.take_sugs;
                    },
                    ID_BUILD_SHOWRES_BTN => {
                        vi.show_at_timeout = !vi.show_at_timeout;
                    },
                    ID_BUILD_VOTEONE_BTN => {
                        vi.vote_once = !vi.vote_once;
                    },
                    ID_BUILD_PING_BTN => {
                        vi.ping_chan += 1;
                        if vi.ping_chan >= 3 {
                            vi.ping_chan = 0;
                        }
                    },
                    ID_BUILD_PROMPT_BTN => {
                        // send modal to get a different duration
                        let current_prompt = vi.prompt.clone();
                        interaction.create_interaction_response(&ctx, |resp| {
                            resp.kind(InteractionResponseType::Modal).interaction_response_data(|d| {
                                d
                                    .custom_id(ID_BUILD_PROMPT_INPUT)
                                    .title("Prompt")
                                    .components(|c| {
                                        c.create_action_row(|r| {
                                            r.create_input_text(|t| {
                                                t
                                                    .custom_id(ID_BUILD_PROMPT_INPUT_TXT)
                                                    .style(InputTextStyle::Short)
                                                    .label("Vote Prompt Message")
                                                    .min_length(0)
                                                    .max_length(24)
                                                    .required(true)
                                                    .value(current_prompt)
                                            })
                                        })
                                    })
                            })
                        }).await.unwrap();

                        update_dm = false;
                    },
                    ID_BUILD_DUR_BTN => {
                        // send modal to get a different duration
                        interaction.create_interaction_response(&ctx, |resp| {
                            resp.kind(InteractionResponseType::Modal).interaction_response_data(|d| {
                                d
                                    .custom_id(ID_BUILD_DUR_INPUT)
                                    .title("Hours Till Timeout")
                                    .components(|c| {
                                        c.create_action_row(|r| {
                                            r.create_input_text(|t| {
                                                t
                                                    .custom_id(ID_BUILD_DUR_INPUT_TXT)
                                                    .style(InputTextStyle::Short)
                                                    .label("# Hours")
                                                    .min_length(1)
                                                    .max_length(6)
                                                    .required(true)
                                                    .value(vi.get_timeout_str(""))
                                            })
                                        })
                                    })
                            })
                        }).await.unwrap();

                        update_dm = false;
                    },
                    ID_BUILD_CHOICE_BTN => {
                        // send modal to edit choices
                        interaction.create_interaction_response(&ctx, |resp| {
                            resp.kind(InteractionResponseType::Modal).interaction_response_data(|d| {
                                d
                                    .custom_id(ID_BUILD_VAL_INPUT)
                                    .title("Vote Choices (one per line)")
                                    .components(|c| {
                                        c.create_action_row(|r| {
                                            r.create_input_text(|mut t| {
                                                t = t
                                                    .custom_id(ID_BUILD_VAL_INPUT_TXT)
                                                    .style(InputTextStyle::Paragraph)
                                                    .label("Choices")
                                                    .min_length(1)
                                                    .max_length(600)
                                                    .required(true);
                                                
                                                if vi.vals.len() > 0 {
                                                    t = t.value(vi.vals.join("\n"));
                                                }

                                                t
                                            })
                                        })
                                    })
                            })
                        }).await.unwrap();

                        update_dm = false;
                    },
                    ID_BUILD_SUBMIT => {
                        println!("Creating vote with options: {:?}", vi);

                        let update_content: String = if vi.take_sugs {
                            "Vote Created\nHit 'Start Vote' in the channel to end the suggestion phase early before the timeout.\nAs vote creator you can edit and remove other's suggestions from there as well with the 'Add Suggestion' button.".into()
                        } else {
                            "Vote Created".into()
                        };

                        // start the vote or the sug phase
                        interaction.create_interaction_response(&ctx, |resp| {
                            resp.kind(InteractionResponseType::UpdateMessage).interaction_response_data(|d| {
                                d.content(update_content).components(|c| c)
                            })
                        }).await.unwrap();

                        // also start the vote
                        do_vote = true;
                        break;
                    },
                    ID_BUILD_CANCEL => {
                        interaction.create_interaction_response(&ctx, |resp| {
                            resp.kind(InteractionResponseType::UpdateMessage).interaction_response_data(|d| {
                                d.content("Canceled").components(|c| c)
                            })
                        }).await.unwrap();

                        break;
                    },
                    _ => {
                        panic!("Got unexpected button id on the dm");
                    }
                }
                if update_dm {
                    interaction.create_interaction_response(&ctx, |resp| {
                        resp.kind(InteractionResponseType::UpdateMessage).interaction_response_data(|d| {
                            d.content(VOTE_DM_CONT).components(|c| {
                                create_dm_vote_comp(c, &vi)
                            })
                        })
                    }).await.unwrap();
                }
            },
            Some(interaction) = mod_col.next() => {
                match &interaction.data.custom_id[..] {
                    ID_BUILD_PROMPT_INPUT => {
                        if let ActionRowComponent::InputText(it) = &interaction.data.components[0].components[0] {
                            vi.prompt = it.value.clone();
                            vi.prompt = content_safe(
                                &ctx,
                                vi.prompt,
                                &ContentSafeOptions::default(),
                                &[]
                            );
                            if (vi.prompt.len() > 0) && (!vi.prompt.ends_with('\n')) {
                                vi.prompt.push('\n');
                            }
                            // sanitize anything else?
                        } else {
                            vi.prompt = "".into()
                        }
                    },
                    ID_BUILD_DUR_INPUT => {
                        if let ActionRowComponent::InputText(it) = &interaction.data.components[0].components[0] {
                            if let Ok(hrs) = it.value.parse::<f64>() {
                                if hrs >= MIN_DUR_HR && hrs <= MAX_DUR_HR {
                                    vi.timeout = Duration::from_secs_f64(hrs * 60.0 * 60.0);
                                } else {
                                    println!("Not accepting bad duration amount");
                                }
                            } else {
                                println!("Not accepting non-number duration amount");
                            }
                        } else {
                            panic!("No input found on timeout option dm modal");
                        }
                    },
                    ID_BUILD_VAL_INPUT => {
                        if let ActionRowComponent::InputText(it) = &interaction.data.components[0].components[0] {
                            let v = content_safe(&ctx, &it.value, &ContentSafeOptions::default(), &[]);
                            vi.vals = v.split('\n').map(|x| String::from(x.trim())).collect();

                        } else {
                            panic!("No input found on val dm modal");
                        }
                    },
                    _ => {
                        panic!("Got unexpected modal id on the dm");
                    }
                }

                // update the dm
                interaction.create_interaction_response(&ctx, |resp| {
                    resp.kind(InteractionResponseType::UpdateMessage).interaction_response_data(|d| {
                        d.content(VOTE_DM_CONT).components(|c| {
                            create_dm_vote_comp(c, &vi)
                        })
                    })
                }).await.unwrap();
            }
            else => {
                println!("Ending collection for dm interactions! Timed out");
                // update the dm to say so
                dm.edit(&ctx, |e| {
                    e.content("Vote creation timed out").components(|c| c)
                }).await.unwrap();
                break;
            }
        }
    } // end select loop

    drop(mod_col);
    drop(dm_col);

    if do_vote {
        if vi.take_sugs {
            // start suggestion path
            handle_suggestion_phase(&ctx, &msg.author, msg.channel_id, vi).await;
        } else {
            // just start the vote
            start_vote(&ctx, msg.channel_id, vi).await
        }
    }

}

struct Handler;

#[async_trait]
impl EventHandler for Handler {
    async fn message(&self, ctx: Context, msg: Message) {
        if msg.content == "letsvote" {
            handle_dm_vote(ctx, msg).await;
        }
    }

    async fn ready(&self, _ctx: Context, _data: Ready) {
        println!("Client Connected");
    }
}

#[tokio::main]
async fn main() {
    let token = env::var("DISCORD_TOKEN").expect("Requires DISCORD_TOKEN var");

    let intents = GatewayIntents::GUILD_MESSAGES | GatewayIntents::DIRECT_MESSAGES | GatewayIntents::MESSAGE_CONTENT;

    let mut client = Client::builder(&token, intents).event_handler(Handler).await.expect("Error creating client");

    if let Err(why) = client.start().await {
        println!("Client error: {:?}", why);
    }
}
