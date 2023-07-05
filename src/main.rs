use std::{env, time::Duration, vec, collections::btree_map::Values};
use serenity::{
    async_trait,
    prelude::*,
    futures::StreamExt,
    model::{
        channel::Message,
        gateway::Ready,
        application::interaction::InteractionResponseType,
        application::component::InputTextStyle,
        prelude::prelude::component::ActionRowComponent,
    },
    collector::ModalInteractionCollectorBuilder,
};

const ID_VOTE_OPTIONS_INPUT: &str = "InputOptions";
const ID_VOTE_TYPE: &str = "VoteKind";

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

        let typestr = VoteType::from_string(&interaction.data.values[0]);
        println!("Type choosen: {:?}", typestr);

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

        //TODO
        // now from the interaction above we can create the vote for everyone in the channel
        //msg.channel_id.send_message(&ctx, |m| {
        //    // create the vote message
        //    create_vote_message(m, vec![], 0);
        //}).await.unwrap();
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
