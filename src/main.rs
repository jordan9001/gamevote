use std::{env, time::Duration};
use serenity::{
    async_trait,
    prelude::*,
    futures::StreamExt,
    model::{
        channel::Message,
        gateway::Ready,
        application::interaction::InteractionResponseType,
        application::component::InputTextStyle,
        prelude::{prelude::component::ActionRowComponent, ChannelId},
    },
    collector::ModalInteractionCollectorBuilder,
};

const ID_VOTE_OPTIONS_INPUT: &str = "InputOptions";
const ID_VOTE_TYPE: &str = "VoteKind";

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

async fn start_vote(ctx: &Context, cid: ChannelId, votetype: VoteType, vals: Vec<&str>, timeout: Duration) {
    // send a message (or multiple) to the channel for everyone, with the voting options
    // depending on the vote, these will be different values to fill in
    // I think there will have to be a modal for voting
    // and an ephemeral message to display your vote
    // and you can change your vote as needed?
    // final submit/view results button will not double submit your vote,
    // but will just show results in ephemeral if you have submitted

    // I will need {user_id1:{choice1: val, choice2: val}, user_id2:{choice1:val, choice2:val}}
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

        // Collect the vote type first
        let interaction = match dm.await_component_interaction(&ctx).timeout(Duration::from_secs(60 * 3)).await {
            Some(x) => x,
            None => {
                dm.reply(&ctx, "Timed out").await.unwrap();
                return;
            }
        };

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
            .filter(|i| -> bool {
                i.data.custom_id == ID_VOTE_OPTIONS_INPUT
            })
            .build();

        let interaction = match collector.next().await {
            Some(x) => x,
            None => {
                dm.reply(&ctx, "Timed out waiting for choices").await.unwrap();
                return;
            }
        };

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

    async fn ready(&self, _ctx: Context, data: Ready) {
        println!("Client Connected: {:?}", data.user);
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
