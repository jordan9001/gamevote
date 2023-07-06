use std::{
    env,
    time::Duration,
    collections::HashMap,
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
    collector::{ModalInteractionCollectorBuilder, ComponentInteractionCollectorBuilder}, builder::CreateComponents,
};

const ID_VOTE_OPTIONS_INPUT: &str = "InputOptions";
const ID_VOTE_TYPE: &str = "VoteKind";
const ID_VOTE_VAL_PREFIX: &str = "VoteVal";
const ID_VOTE_BTN: &str = "MainVoteBtn";
const ID_VOTE_LEFT: &str = "VoteLeft";
const ID_VOTE_RIGHT: &str = "VoteRight";
const ID_VOTE_SUBMIT: &str = "VoteSubmit";

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60*60*9);

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
            _ => "Unknown".into(),
        }
    }
}

impl VoteType {
    fn from_string(s: &String) -> Self {
        match &s[..] {
            "Approval" => VOTE_APPROVAL,
            "Score" => VOTE_SCORE,
            "Borda" => VOTE_BORDA,
            _ => VoteType(0),
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
}

struct UserVote {
    votes: CastVotes,
    fullvote: bool,
    votemsg: MessageId,
    page: usize,
}

struct Vote {
    kind: VoteType,
    uservotes: HashMap<UserId,UserVote>,
}

impl Vote {
    fn new(vt: VoteType) -> Self {
        Vote {
            kind: vt,
            uservotes: HashMap::new(),
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

fn create_user_message<'a, 'b, 'c>(mut c: &'b mut CreateComponents, vals: &'c Vec<&'a str>, page: usize, vote: &'c Vote) -> &'b mut CreateComponents {
    let i = page * 4;
    for j in 0..4 {
        let vali = i + j;
        if vali >= vals.len() {
            break;
        }
        c = c.create_action_row(|r| {
            r.create_button(|btn| {
                btn.custom_id(format!("{}{}", ID_VOTE_VAL_PREFIX, vali))
                    .style(ButtonStyle::Secondary)
                    .label(vals[vali]) // TODO adjust label accoring to vote
            })
        });
    }
    // add a row for the movement and submit buttons
    c.create_action_row(|r| {
        r
            .create_button(|btn| {
                btn.custom_id(ID_VOTE_LEFT)
                    .style(ButtonStyle::Secondary)
                    .label("<")
            })
            .create_button(|btn| {
                btn.custom_id(ID_VOTE_RIGHT)
                    .style(ButtonStyle::Secondary)
                    .label(">")
            })
            .create_button(|btn| {
                btn.custom_id(ID_VOTE_SUBMIT)
                    .style(ButtonStyle::Primary)
                    .label("Submit")
            })
    })
}

async fn start_vote(ctx: &Context, cid: ChannelId, votetype: VoteType, vals: Vec<&str>, timeout: Duration) {
    let mut vote = Vote::new(votetype);
    let num_pages = ((vals.len() -1) / 5) + 1;

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

    // collector for the vote buttons
    //TODO
    
    // collector for the left, right, submit buttons
    let mut nav_col = ComponentInteractionCollectorBuilder::new(ctx)
        .timeout(timeout)
        .channel_id(cid)
        .filter(|i| {
            //DEBUG
            // filter by our ephemeral message ids? or instead mark our components with a voteid?
            //TODO
            // filter by component ids for these buttons
            //TODO
            false
        })
        .build();

    // collector for the modals from some votes
    //TODO

    loop {
        tokio::select! {
            Some(interaction) = mainbtn_col.next() => {
                println!("Got main btn interaction");
                let mut page = 0;
                let uid = interaction.user.id;
                // lookup user
                if let Some(uv) = vote.uservotes.get(&uid) {
                    page = uv.page;
                }

                // this button brings up a new ephemeral message for voting
                interaction.create_interaction_response(ctx, |resp| {
                    resp.kind(InteractionResponseType::ChannelMessageWithSource).interaction_response_data(|d| {
                        d
                            .content(format!("Page {}/{}", page+1, num_pages))
                            .components(|c| {
                                create_user_message(c, &vals, page, &vote)
                            })
                            .ephemeral(true)
                    })
                }).await.unwrap();

                // get the response info
                let respmsg = interaction.get_interaction_response(ctx).await.unwrap();

                if let Some(uv) = vote.uservotes.get_mut(&uid) {
                    // delete any old ephemeral message
                    //TODO
                    // set the message id from the new ephemeral message
                    uv.votemsg = respmsg.id
                } else {
                    // create uservote entry and record the ephemeral message id
                    vote.uservotes.insert(uid, UserVote{
                        votes: CastVotes::new(votetype),
                        fullvote: false,
                        votemsg: respmsg.id,
                        page: 0,
                    });
                }
            },
            Some(interaction) = nav_col.next() => {
                println!("Got nav interaction");
                //TODO
            },
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
        //TODO test short timeout, see if collectors stop in select right
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
