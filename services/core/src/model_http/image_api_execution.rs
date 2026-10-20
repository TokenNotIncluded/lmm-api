//! Public native-Key images retain the original Key's actor, owner, group,
//! payer ordering and budget. They never use drawing's personal Session source.
use super::*;
use crate::model_http::image_provider::{ImageMediaType, ImageMultipartInput};
use std::net::IpAddr;

/// Host-selected persisted identity, never deserialized from model/Go input.
#[derive(Clone, Copy, Debug, Serialize, Eq, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum SourceKey {
    Drawing { credential_id: i64 },
    ApiKey { credential_id: i64 },
}
fn upload_digest(edits: &ImageMultipartInput) -> Value {
    let item = |i: &crate::model_http::image_provider::ImageUpload| json!({"sha256":hex(&Sha256::digest(&i.bytes)),"media_type":match i.media_type{ImageMediaType::Png=>"image/png",ImageMediaType::Jpeg=>"image/jpeg",ImageMediaType::Webp=>"image/webp"}});
    json!({"images":edits.images.iter().map(item).collect::<Vec<_>>(),"mask":edits.mask.as_ref().map(item)})
}
impl ImageService {
    pub(crate) async fn execute_api(
        &self,
        secret: &str,
        request_key: &str,
        groups: &[String],
        input: DrawingInput,
        edits: Option<ImageMultipartInput>,
        peer: Option<IpAddr>,
    ) -> Result<Receipt> {
        self.execute_api_with_response_format(
            secret,
            request_key,
            groups,
            input,
            edits,
            peer,
            "b64_json",
        )
        .await
    }
    pub(crate) async fn execute_api_with_response_format(
        &self,
        secret: &str,
        request_key: &str,
        groups: &[String],
        input: DrawingInput,
        edits: Option<ImageMultipartInput>,
        peer: Option<IpAddr>,
        response_format: &str,
    ) -> Result<Receipt> {
        if !matches!(response_format, "" | "b64_json" | "url") {
            return Err(Error::Invalid);
        }
        let response_format = response_format.to_owned();
        let service = self.clone();
        let secret = secret.to_owned();
        let key = request_key.to_owned();
        let groups = groups.to_vec();
        tokio::spawn(async move {
            service
                .execute_api_owned(&secret, &key, &groups, input, edits, peer, &response_format)
                .await
        })
        .await
        .map_err(|_| Error::Pending)?
    }
    async fn execute_api_owned(
        &self,
        secret: &str,
        request_key: &str,
        groups: &[String],
        mut input: DrawingInput,
        edits: Option<ImageMultipartInput>,
        peer: Option<IpAddr>,
        response_format: &str,
    ) -> Result<Receipt> {
        let _permit = self
            .gate
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Pending)?;
        if !input.group.is_empty()
            || request_key.len() != 64
            || !request_key.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(Error::Invalid);
        }
        if input.n == 0 {
            input.n = 1;
        }
        let endpoint = if edits.is_some() {
            ImageEndpoint::Edits
        } else {
            ImageEndpoint::Generations
        };
        let (group, quote) = groups
            .iter()
            .find_map(|g| {
                input.group = g.clone();
                self.quote_for(&input, endpoint)
                    .ok()
                    .map(|q| (g.clone(), q))
            })
            .ok_or(Error::Forbidden)?;
        input.group = group;
        let metadata = edits.as_ref().map(upload_digest);
        let mut tx = begin(&self.pool).await?;
        let actor = identity::authorize_in_transaction(&mut tx, secret)
            .await
            .map_err(identity_error)?;
        if actor.credential_kind != CredentialKind::ApiKey {
            return Err(Error::Forbidden);
        }
        // The canonical Key model/group/IP policy is rechecked again in the
        // same transaction that reserves and claims the actual provider send.
        let source = SourceKey::ApiKey {
            credential_id: actor.credential_id,
        };
        let id = format!("image:{request_key}");
        let frozen = price(&quote);
        let fingerprint = digest(&json!([source, input, metadata, response_format, frozen]))?;
        let version: i64 = sqlx::query_scalar(
            "SELECT auth_version FROM core_identity.users WHERE id=$1 FOR SHARE",
        )
        .bind(actor.user_id)
        .fetch_one(&mut *tx)
        .await?;
        let at = now(&mut tx).await?;
        sqlx::query("INSERT INTO core_image.operations(id,source_kind,key_id,actor_user_id,owner_account_id,auth_version,input,fingerprint,price,state,created_at,expires_at,confirmed_at) VALUES($1,'api_key',$2,$3,$4,$5,$6,$7,$8,'confirmed',$9,$9+600,$9) ON CONFLICT(id) DO NOTHING")
            .bind(&id).bind(actor.credential_id).bind(actor.user_id).bind(actor.owner_account_id).bind(version).bind(json!(input)).bind(fingerprint.as_slice()).bind(json!(frozen)).bind(at).execute(&mut *tx).await?;
        let r = sqlx::query(
            "SELECT fingerprint,state,key_id FROM core_image.operations WHERE id=$1 FOR UPDATE",
        )
        .bind(&id)
        .fetch_one(&mut *tx)
        .await?;
        if r.try_get::<Vec<u8>, _>("fingerprint")? != fingerprint
            || r.try_get::<Option<i64>, _>("key_id")? != Some(actor.credential_id)
        {
            return Err(Error::Conflict);
        }
        let state: String = r.try_get("state")?;
        tx.commit().await.map_err(|_| Error::Pending)?;
        if state != "confirmed" {
            return self.query_api_with_peer(secret, &id, peer).await;
        }
        let provider = self
            .providers
            .get(&quote.binding().upstream_id)
            .ok_or(Error::Invalid)?;
        let prepared = match edits {
            None => {
                provider
                    .prepare_with_response_format(&quote, &input.prompt, response_format)
                    .await
            }
            Some(files) => {
                provider
                    .prepare_edits_with_response_format(
                        &quote,
                        &input.prompt,
                        files,
                        response_format,
                    )
                    .await
            }
        }
        .map_err(|_| Error::Invalid)?;
        let worker = random()?;
        let spec = Request {
            id: id.clone(),
            fingerprint,
            price: frozen,
            reserve: quote.reservation_credits().into(),
            lease_seconds: 300,
            maximum_seconds: 300,
        };
        let mut tx = begin(&self.pool).await?;
        self.charging
            .reserve_image_key(&mut tx, secret, &id, &spec, &worker, peer)
            .await?;
        self.charging
            .start_image_key(&mut tx, secret, &id, &worker, peer)
            .await?;
        tx.commit().await.map_err(|_| Error::Pending)?;
        let result=async {
            let observation=prepared.dispatch().await.map_err(|_|Error::Pending)?;
            let image=ImageUsage{output_images:observation.output_images,provider_receipt_sha256:observation.provider_receipt_sha256};
            quote.settlement(image.output_images).map_err(|_|Error::Pending)?;
            let output_digest=digest(&observation.data)?;
            let mut tx=begin(&self.pool).await?;
            let changed=sqlx::query("UPDATE core_image.operations SET state='billing_pending',output=$2,output_digest=$3,image_usage=$4 WHERE id=$1 AND state='dispatching' AND charge_id=$1 AND output IS NULL").bind(&id).bind(observation.data).bind(output_digest.as_slice()).bind(json!(image)).execute(&mut *tx).await?.rows_affected();
            if changed!=1{return Err(Error::Pending)}
            tx.commit().await.map_err(|_|Error::Pending)?;
            let charge=self.charging.settle(&id,&worker,Usage{image:Some(image),..Usage::default()},Outcome::Completed).await?;
            self.complete(&charge).await?;
            self.query_api_with_peer(secret,&id,peer).await
        }.await;
        if result.is_err() {
            self.charging.mark_unknown(&id, &worker).await?;
        }
        result
    }
    pub(crate) async fn query_api_with_peer(
        &self,
        secret: &str,
        id: &str,
        peer: Option<IpAddr>,
    ) -> Result<Receipt> {
        if !valid_id(id) {
            return Err(Error::Invalid);
        }
        let mut tx = begin(&self.pool).await?;
        let a = identity::authorize_in_transaction(&mut tx, secret)
            .await
            .map_err(identity_error)?;
        if a.credential_kind != CredentialKind::ApiKey {
            return Err(Error::Forbidden);
        }
        let r=sqlx::query("SELECT * FROM core_image.operations WHERE id=$1 AND source_kind='api_key' AND key_id=$2 AND actor_user_id=$3 AND owner_account_id=$4 FOR SHARE").bind(id).bind(a.credential_id).bind(a.user_id).bind(a.owner_account_id).fetch_optional(&mut *tx).await?.ok_or(Error::Missing)?;
        let frozen: Price =
            serde_json::from_value(r.try_get("price")?).map_err(|_| Error::Storage)?;
        identity::authorize_model_request_in_transaction(
            &mut tx,
            secret,
            &frozen.model,
            peer,
            &frozen.group,
        )
        .await
        .map_err(identity_error)?;
        let out = receipt_row(&r)?;
        tx.commit().await.map_err(|_| Error::Pending)?;
        Ok(out)
    }
}
