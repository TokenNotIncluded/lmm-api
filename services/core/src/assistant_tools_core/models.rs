//! A read-only surface injected from the actual serving ModelGateway. No tool
//! input can supply a route release, provider URL, credential, payer or price.
use super::{Error, context::VerifiedToolContext};
use crate::{model_http::ModelGateway, routing::Router};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgConnection;
use std::{collections::BTreeSet, sync::Arc};
#[derive(Clone)]
pub struct ModelsProjection {
    routing: Arc<Router>,
    surface: Arc<crate::model_http::CatalogSurface>,
}
#[derive(Serialize)]
pub struct AvailableModels {
    pub actor_user_id: i64,
    pub exact_model_ids: Vec<String>,
    pub developer_access_granted: bool,
    pub locked: bool,
    pub public_preview: bool,
    pub key_group_options: Vec<String>,
    pub catalogue: Option<crate::model_http::CatalogV1>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelPricingArgs {
    pub model_id: String,
    pub group: Option<String>,
}
impl ModelsProjection {
    pub async fn available_tool_in_transaction(
        &self,
        c: &mut PgConnection,
        context: &VerifiedToolContext,
    ) -> Result<Value, Error> {
        let facts = self.read_in_transaction(c, context).await?;
        Ok(
            json!({"ok":!facts.exact_model_ids.is_empty(),"status":if facts.exact_model_ids.is_empty(){"catalog_unavailable"}else if facts.public_preview{"public_preview"}else{"available"},"model_ids":facts.exact_model_ids,"developer_access_granted":facts.developer_access_granted,"account_model_access_locked":facts.locked,"public_preview":facts.public_preview,"key_group_options":facts.key_group_options,"catalog_source":"core serving model catalogue","availability_scope":if facts.public_preview{"current_public_model_policy"}else{"current_account_routing_groups"}}),
        )
    }
    pub async fn pricing_in_transaction(
        &self,
        c: &mut PgConnection,
        context: &VerifiedToolContext,
        args: &ModelPricingArgs,
    ) -> Result<Value, Error> {
        if !(1..=200).contains(&args.model_id.chars().count())
            || args.model_id.chars().any(char::is_control)
            || args
                .group
                .as_ref()
                .is_some_and(|g| g.chars().count() > 64 || g.chars().any(char::is_control))
        {
            return Err(Error::Invalid);
        }
        let available = self.read_in_transaction(c, context).await?;
        if available.locked {
            return Err(Error::Forbidden);
        }
        let catalogue = available.catalogue.ok_or(Error::Unavailable)?;
        let rows: Vec<_> = catalogue
            .models
            .into_iter()
            .filter(|m| {
                m.name == args.model_id
                    && args
                        .group
                        .as_ref()
                        .is_none_or(|g| g.is_empty() || g == &m.group)
            })
            .collect();
        if rows.is_empty() {
            return Err(Error::NotFound);
        }
        // The serving catalogue already applies group/speed multipliers. Null
        // cost dimensions remain unknown; no float arithmetic or money hold.
        Ok(
            json!({"ok":true,"model_id":args.model_id,"prices":rows,"routing_version":catalogue.routing_version,"pricing_version":catalogue.pricing_version,"source":"core serving model catalogue","group_multiplier_already_applied":true,"price_locked":false,"upstream_cost_available":false}),
        )
    }
    pub fn from_gateway(gateway: &ModelGateway) -> Self {
        Self {
            routing: gateway.dashboard_routing(),
            surface: gateway.oauth_catalog_surface(),
        }
    }
    pub async fn read_in_transaction(
        &self,
        c: &mut PgConnection,
        context: &VerifiedToolContext,
    ) -> Result<AvailableModels, Error> {
        let actor = context.revalidate(c).await?;
        let candidates:Vec<String>=sqlx::query_scalar("SELECT group_name FROM core_identity.user_routing_groups WHERE user_id=$1 AND enabled ORDER BY group_name").bind(actor.user).fetch_all(&mut *c).await?;
        let groups: Vec<String> = candidates
            .into_iter()
            .filter(|g| self.routing.catalog(g).is_ok())
            .collect();
        let public = crate::abuse::public_models_in_transaction(c, actor.user)
            .await
            .map_err(|_| Error::Unavailable)?;
        if groups.is_empty() || (!actor.developer && public.is_none()) {
            return Ok(AvailableModels {
                actor_user_id: actor.user,
                exact_model_ids: vec![],
                developer_access_granted: actor.developer,
                locked: true,
                public_preview: public.is_some(),
                key_group_options: vec![],
                catalogue: None,
            });
        }
        let mut catalogue = self
            .surface
            .project(
                &groups,
                "urn:lmm:core-assistant-catalog",
                chrono::Utc::now()
                    .timestamp()
                    .try_into()
                    .map_err(|_| Error::Unavailable)?,
            )
            .map_err(|_| Error::Unavailable)?;
        if let Some(public) = &public {
            catalogue.models.retain(|m| public.contains(&m.name))
        }
        let ids: BTreeSet<String> = catalogue.models.iter().map(|m| m.name.clone()).collect();
        Ok(AvailableModels {
            actor_user_id: actor.user,
            exact_model_ids: ids.into_iter().collect(),
            developer_access_granted: actor.developer,
            locked: false,
            public_preview: public.is_some(),
            key_group_options: if actor.developer { groups } else { vec![] },
            catalogue: Some(catalogue),
        })
    }
}
