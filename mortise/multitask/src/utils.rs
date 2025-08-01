use crate::config::Database;
use mongodb::bson::oid::ObjectId;
use mongodb::{options::ClientOptions, Client};
use polars::prelude::{CsvWriter, DataFrame, SerWriter};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Serialize, Deserialize)]
pub struct StoreData {
    #[serde(rename = "_id")]
    id: ObjectId,
    config: String,
    desc: String,
    trace: serde_json::Value,
    data: serde_json::Value,
}

pub fn write_df_csv<P>(df: &mut DataFrame, output_path: P) -> anyhow::Result<()>
where
    P: AsRef<Path>,
{
    let mut file = std::fs::File::create(output_path)?;
    CsvWriter::new(&mut file).finish(df)?;
    Ok(())
}

pub async fn save_to_db<P, S, S1, S2>(
    df: &mut DataFrame,
    db: &Database,
    collection_name: S,
    config: S1,
    desc: S2,
    trace_config_path: P,
) -> anyhow::Result<()>
where
    S: AsRef<str>,
    S1: ToString,
    S2: ToString,
    P: AsRef<Path>,
{
    let client_options = ClientOptions::parse(&db.url).await?;
    let client = Client::with_options(client_options)?;
    let db = client.database(&db.name);
    let collection = db.collection::<StoreData>(collection_name.as_ref());
    let trace_config = serde_json::from_reader(std::fs::File::open(trace_config_path)?)?;
    let document = StoreData {
        id: ObjectId::new(),
        config: config.to_string(),
        desc: desc.to_string(),
        trace: trace_config,
        data: serde_json::to_value(df.get_columns())?,
    };
    collection.insert_one(document).await?;
    Ok(())
}
