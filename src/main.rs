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
        prelude::{prelude::component::ActionRowComponent, ChannelId, UserId, MessageId},
    },
    collector::{ModalInteractionCollectorBuilder, ComponentInteractionCollectorBuilder},
    builder::CreateComponents,
};
use tallystick::{
    approval::DefaultApprovalTally,
    borda::DefaultBordaTally,
    score::ScoreTally,
};

const ID_VOTE_OPTIONS_INPUT: &str = "InputOptions";
const ID_VOTE_TYPE: &str = "VoteKind";
const ID_VOTE_VAL_PREFIX: &str = "VoteVal";
const ID_VOTE_VAL_INPUT: &str = "ValInModal";
const ID_VOTE_VAL_INPUT_PREFIX: &str = "ValIn";
const ID_VOTE_BTN: &str = "MainVoteBtn";
const ID_VOTE_LEFT: &str = "VoteLeft";
const ID_VOTE_RIGHT: &str = "VoteRight";
const ID_VOTE_SUBMIT: &str = "VoteSubmit";

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60*60*9);
const PERPAGE: usize = 4;

#[derive(PartialEq, Eq, Debug, Clone, Copy)]
struct VoteType(u32);
const VOTE_APPROVAL: VoteType   = VoteType(1 << 0);
const VOTE_SCORE: VoteType      = VoteType(1 << 1);
const VOTE_BORDA: VoteType      = VoteType(1 << 2);

impl ToString for VoteType {
    fn to_string(&self) -> String {
        match *self {
            VOTE_APPROVAL => "Approval".into(),
            VOTE_SCORE => "Score".into(),
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
            "Borda" => VOTE_BORDA,
            _ => panic!("Tried to get a vote type from an unknown string! {}", s),
        }
    }

    fn value_name(&self) -> String {
        match *self {
            VOTE_APPROVAL => "choice".into(),
            VOTE_SCORE => "score (-10.0 to 10.0)".into(),
            VOTE_BORDA => "rank (1 is 1st choice, 2 second, ...)".into(),
            _ => panic!("Tried to get value name for unknown vote type! {:?}", self),
        }
    }

    fn is_bad_value(&self, v: f32, vals: &Vec<&str>) -> bool {
        match *self {
            VOTE_APPROVAL => false,
            VOTE_SCORE => v < -10.0 || v > 10.0,
            VOTE_BORDA => v.fract() != 0.0 || v <= 0.0 || v > (vals.len() as f32),
            _ => panic!("Tried to test value for unknown vote type! {:?}", self),
        }
    }
}

enum CastVotes {
    Select(Vec<usize>), // one or more choices, used for normal or approval voting
    Score(HashMap<usize, f32>), // choices associated with a value, also used for rank voting
}

impl CastVotes {
    fn new(vt: VoteType) -> Self {
        match vt {
            VOTE_APPROVAL => CastVotes::Select(Vec::new()),
            VOTE_SCORE => CastVotes::Score(HashMap::new()),
            VOTE_BORDA => CastVotes::Score(HashMap::new()),
            _ => panic!("Tried to create CastVotes with unknown vote type"),
        }
    }

    fn get_vote_vec(&self) -> Vec<usize> {
        // if this is a score, then order them from lowest to highest
        // otherwise just return the vec
        match self {
            CastVotes::Select(v) => {
                v.to_vec()
            },
            CastVotes::Score(m) => {
                let mut vt: Vec<(usize, f32)> = Vec::new();
                for (u, f) in m {
                    vt.push((*u,*f));
                }
                vt.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
                vt.iter().map(|x| x.0).collect()
            }
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
        }
    }

    fn are_valid(&self, vt: VoteType, size: usize) -> bool {
        match vt {
            VOTE_APPROVAL => true,
            VOTE_SCORE => true,
            VOTE_BORDA => {
                // check every key is in there
                // check each key has a non-zero rating
                // not too worried about tie ratings, just let the sorting sort it
                match self {
                    CastVotes::Select(_) => {
                        panic!("Borda vote with select backing");
                    },
                    CastVotes::Score(m) => {
                        for i in 0..size {
                            if let Some(v) = m.get(&i) {
                                if *v < 1.0 {
                                    return false;
                                }
                            } else {
                                return false;
                            }

                        }
                    },
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
    ($tally:expr, $vals:expr, $votetype:expr, $num_voters:expr) => {
    {
        let mut result: String = format!("{} Vote Results (so far: {} voters):\nWinner:\n", $votetype.to_string(), $num_voters);
        let winners = $tally.winners().all();
        for w in winners {
            result.push_str(&format!("{}\n", $vals[w]));
        }
        result.push_str("\nTotals:\n");
        for (w, c) in $tally.totals() {
            result.push_str(&format!("{}: {}\n", c, $vals[w]));
        }
        result.push_str("\n Submit again to refresh your results");
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

    fn get_results(&self, vals: &Vec<&str>) -> String {
        let mut num_voters = 0;
        match self.kind {
            VOTE_APPROVAL => {
                let mut tally = DefaultApprovalTally::new(1);

                for (_, cv) in &self.submittedvotes {
                    tally.add(cv.get_vote_vec());
                    num_voters += 1;
                }

                tally_str!(tally, vals, self.kind, num_voters)
            },
            VOTE_SCORE => {
                let mut tally = ScoreTally::<usize, f32>::new(1);

                for (_, cv) in &self.submittedvotes {
                    tally.add(cv.get_vote_weight_vec());
                    num_voters += 1;
                }

                tally_str!(tally, vals, self.kind, num_voters)
            },
            VOTE_BORDA => {
                let mut tally = DefaultBordaTally::new(1, tallystick::borda::Variant::Borda);

                for (_, cv) in &self.submittedvotes {
                    tally.add(cv.get_vote_vec()).unwrap();
                    num_voters += 1;
                }

                tally_str!(tally, vals, self.kind, num_voters)
            },
            _ => panic!("Tried to get results with unknown vote type"),
        }
    }
}

// a macro because the builder for creating and editing have the same functions, but different types
// maybe serenity should put those in a trait
macro_rules! setup_base_message {
    ($m:expr, $num_votes:expr, $vtype:expr) => {
        $m
            .content(format!("{} Vote: {} Votes so far", $vtype, $num_votes))
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

fn create_user_message<'a, 'b, 'c>(mut c: &'b mut CreateComponents, vals: &'c Vec<&'a str>, page: usize, vote: &'c Vote, uid: UserId) -> &'b mut CreateComponents {
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
    let donav = vals.len() > PERPAGE && page == 0;
    c.create_action_row(|mut r| {
        if donav {
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
    ($interaction:expr, $uid:expr, $extra:expr, $vote:expr, $ctx:expr, $num_pages:expr, $vals:expr, $first:expr) => {
        // edit or create the ephemeral
        let irkind: InteractionResponseType = if $first {
            InteractionResponseType::ChannelMessageWithSource
        } else {
            InteractionResponseType::UpdateMessage
        };

        let disppage = {
            let rvote = $vote.read().unwrap();

            if let Some(uv) = rvote.uservotes.get(&$uid) {
                uv.page
            } else {
                panic!("Tried to get user page for user not in vote structure yet");
            }
        };

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
    };
}

async fn start_vote(ctx: &Context, cid: ChannelId, votetype: VoteType, vals: Vec<&str>, timeout: Duration) {
    let vote = Arc::new(RwLock::new(Vote::new(votetype)));
    let num_pages = ((vals.len() -1) / PERPAGE) + 1;

    // actually let's try just having a "vote" button, so we can edit the ephemeral button to match each user
    let basemsg = cid.send_message(ctx, |m| {
        setup_base_message!(m, 0, votetype.to_string())
    }).await.unwrap();

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
                user_vote_message!(interaction, uid, "", vote, ctx, num_pages, vals, true);

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
                                if dir {
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
                        user_vote_message!(interaction, uid, "", vote, ctx, num_pages, vals, false);
                    },
                    ID_VOTE_SUBMIT => {
                        // submit the vote for this user, if we can
                        // first check that it is a valid submission, and let them know if it is not
                        let valid_submission;


                        let mut isfirst = false;
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
                            let errresp = format!("\nError: invalid values for a {} vote, please fix your vote", votetype.to_string());
                            user_vote_message!(interaction, uid, errresp, vote, ctx, num_pages, vals, false);
                        } else {
                            // update the count
                            if isfirst {
                                cid.edit_message(ctx, basemsg.id, |e| {
                                    setup_base_message!(e, subcount, votetype.to_string())
                                }).await.unwrap();
                            }

                            // calculate the vote result
                            let resultsmsg;
                            {
                                let rvote = vote.read().unwrap();

                                resultsmsg = rvote.get_results(&vals);
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
                        let mut current_score: f32 = 0.0;
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
                                            current_score = *score;
                                        }
                                        // we need to drop the lock before we await and create the modal
                                        // don't update the message yet, we will do that after the modal
                                        refresh_msg = false;
                                    },
                                }
                            } else {
                                panic!("Somehow got a vote interaction without an entry in the vote map?")
                            }
                        }

                        if !refresh_msg {
                            interaction.create_interaction_response(ctx, |resp| {
                                resp.kind(InteractionResponseType::Modal).interaction_response_data(|d| {
                                    d.custom_id(ID_VOTE_VAL_INPUT)
                                        .title(format!("Vote for {}", vals[num]))
                                        .components(|c| {
                                            c.create_action_row(|r| {
                                                // create text input for adding the options
                                                r.create_input_text(|t| {
                                                    t.custom_id(format!("{}{}", ID_VOTE_VAL_INPUT_PREFIX, num))
                                                        .style(InputTextStyle::Short)
                                                        .label(votetype.value_name())
                                                        .value(current_score.to_string())
                                                        .min_length(1)
                                                        .max_length(5)
                                                        .required(true)
                                                })
                                            })
                                        })
                                })
                            }).await.unwrap();
                        } else {
                            user_vote_message!(interaction, uid, "", vote, ctx, num_pages, vals, false);
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
                            }
                        } else {
                            panic!("Somehow got a modal interaction without an entry in the vote map?")
                        }
                    }

                    // edit the ephemeral
                    user_vote_message!(interaction, uid, errresp, vote, ctx, num_pages, vals, false);
                } else {
                    panic!("Modal response didn't have input text?");
                }
            }
            else => {
                println!("Ending collection for vote! Timed out?");
                break
            }
        };
    }

}

struct Handler;

#[async_trait]
impl EventHandler for Handler {
    async fn message(&self, ctx: Context, msg: Message) {
        if msg.content != "letsvote" {
            return;
        }

        let dm = msg.author.direct_message(&ctx, |m| {
            m.content("Choose a Vote Type:").components(|c| {
                c.create_action_row(|r| {
                    // create select menu for selecting the type of vote
                    r.create_select_menu(|u| {
                        u.custom_id(ID_VOTE_TYPE)
                            .min_values(1)
                            .max_values(1)
                            .options(|o| {
                                o.create_option(|p| {
                                    p.label(VOTE_APPROVAL)
                                        .value(VOTE_APPROVAL)
                                })
                                .create_option(|p| {
                                    p.label(VOTE_SCORE)
                                        .value(VOTE_SCORE)
                                })  
                                .create_option(|p| {
                                    p.label(VOTE_BORDA)
                                        .value(VOTE_BORDA)
                                })  
                            })

                    })
                })
            })
        }).await.unwrap();

        //DEBUG
        //println!("dm: {:?}", dm);

        // Collect the vote type first
        let interaction = match dm.await_component_interaction(&ctx).timeout(Duration::from_secs(60 * 3)).await {
            Some(x) => x,
            None => {
                dm.reply(&ctx, "Timed out").await.unwrap();
                return;
            }
        };

        //DEBUG
        //println!("vote type int: {:?}", interaction);

        let votetype = VoteType::from_string(&interaction.data.values[0]);
        println!("Type choosen: {:?}", votetype);

        interaction.create_interaction_response(&ctx, |r| {
            r.kind(InteractionResponseType::Modal).interaction_response_data(|d| {
                d.custom_id(ID_VOTE_OPTIONS_INPUT)
                    .title("Comma Separated Vote Choices")
                    .components(|c| {
                        c.create_action_row(|r| {
                            // create text input for adding the options
                            r.create_input_text(|t| {
                                t.custom_id(ID_VOTE_OPTIONS_INPUT)
                                    .style(InputTextStyle::Paragraph)
                                    .label("Choices")
                                    .min_length(2)
                                    .max_length(600)
                                    .required(true)
                            })
                        })
                    })
            })
        }).await.unwrap();

        // wait again for the next interaction
        let mut collector = ModalInteractionCollectorBuilder::new(&ctx)
            .collect_limit(1)
            .timeout(Duration::from_secs(60*9))
            .filter(move |i| -> bool {
                if i.data.custom_id != ID_VOTE_OPTIONS_INPUT {
                    return false;
                }
                if let Some(m) = &i.message {
                    return m.id == dm.id; // make sure it is the interaction for our DM's modal
                } else {
                    return false;
                }
            })
            .build();

        // TODO this is all a bit brittle
        // we need to handle the case where we click away, then want to get the modal back
        // or at least update the message to say, "Cancled"

        let interaction = match collector.next().await {
            Some(x) => x,
            None => {
                dm.reply(&ctx, "Timed out waiting for choices").await.unwrap();
                return;
            }
        };

        //DEBUG
        //println!("modal submit int: {:?}", interaction);

        if interaction.data.components.len() < 1 {
            interaction.create_interaction_response(&ctx, |r| {
                r.kind(InteractionResponseType::UpdateMessage).interaction_response_data(|d| {
                    d.content(format!("Error, no response")).components(|c| c)
                })
            }).await.unwrap();
            return;
        }

        let row = &interaction.data.components[0];
        if row.components.len() < 1 {
            interaction.create_interaction_response(&ctx, |r| {
                r.kind(InteractionResponseType::UpdateMessage).interaction_response_data(|d| {
                    d.content(format!("Error, empty response row")).components(|c| c)
                })
            }).await.unwrap();
            return;
        }

        
        let valstr = match &row.components[0] {
            ActionRowComponent::InputText(txt) => &txt.value,
            _ => {
                interaction.create_interaction_response(&ctx, |r| {
                    r.kind(InteractionResponseType::UpdateMessage).interaction_response_data(|d| {
                        d.content(format!("Choices are required")).components(|c| c)
                    })
                }).await.unwrap();
                return;
            }
        };

        let vals: Vec<&str> = valstr.split(',').map(|x| x.trim()).collect();
        println!("Choices: {:?}", vals);

        // update dm to show it submitted
        interaction.create_interaction_response(&ctx, |r| {
            r.kind(InteractionResponseType::UpdateMessage).interaction_response_data(|d| {
                d.content(format!("Vote Created")).components(|c| c)
            })
        }).await.unwrap();

        // now from the interaction above we can create the vote for everyone in the channel
        start_vote(&ctx, msg.channel_id, votetype, vals, DEFAULT_TIMEOUT).await;

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
