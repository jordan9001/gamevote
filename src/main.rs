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
    collector::ModalInteractionCollectorBuilder,
};

const ID_VOTE_OPTIONS_INPUT: &str = "InputOptions";
const ID_VOTE_TYPE: &str = "VoteKind";
const ID_VOTE_VAL_PREFIX: &str = "VoteVal";

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60*60*9);

#[derive(PartialEq, Eq, Debug)]
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
    Score(Vec<(usize, f32)>), // choices associated with a value, also used for rank voting
}

struct Vote {
    kind: VoteType,
    uservotes: HashMap<UserId,(CastVotes, MessageId)>,
}

impl Vote {
    fn new(vt: VoteType) -> Self {
        Vote {
            kind: vt,
            uservotes: HashMap::new(),
        }
    }
}

async fn start_vote(ctx: &Context, cid: ChannelId, votetype: VoteType, vals: Vec<&str>, timeout: Duration) {
    // send a message (or multiple) to the channel for everyone, with the voting options
    // depending on the vote, these will be different values to fill in
    // only 5 rows per message, so multiple messages might be needed
    // 5 buttons can be per row, but that seems messy
    // modal shows the thing
    // and an ephemeral message to display your vote
    // and you can change your vote as needed?
    // final submit/view results button will not double submit your vote,
    // but will just show results in ephemeral again

    let mut i = 0;
    let num_msg = ((vals.len() -1) / 5) + 1;
    let vlen = vals.len();
    while i < vlen {
        cid.send_message(ctx, |m| {
            m
                .content(format!("{} Vote {}/{}", votetype.to_string(), (i.saturating_sub(1)/5) + 1, num_msg))
                    // TODO, maybe no content except on the first one?
                .components(|mut c| {
                    for j in 0..5 {
                        let vali = i + j;
                        if vali >= vlen {
                            break;
                        }
                        c = c.create_action_row(|r| {
                            r.create_button(|btn| {
                                btn.custom_id(format!("{}{}", ID_VOTE_VAL_PREFIX, vali))
                                    .style(ButtonStyle::Secondary)
                                    .label(vals[vali])
                            })
                        });
                    }
                    c
                })
        }).await.unwrap();
        i += 5;
    }

    // display messages with vote buttons
    let vote = Vote::new(votetype);

    // keep listening for interactions on all the button for this vote
    // if clicked, show a modal that:
    //    shows previous data, if any
    //    has an input where they can give input based on voting kind
    // at the same time we need to listen for submitions coming from those modals
    // make sure to only collect modal interactions associated with this vote here
    // update the user's voting data accordingly
    // update the vote results accordingly, if the user's data is finished
    // and display/update the ephemeral message for the user
    // use interaction.create_followup_message / edit_followup_message? 
    // (Will edit work if a ephemeral msg times out?)
    //TODO
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
