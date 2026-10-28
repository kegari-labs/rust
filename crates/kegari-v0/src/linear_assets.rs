//! Authenticated readback of an exact owned Linear recording publication.
use super::*;
use crate::control::ReceiptPhase;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinearAssetReference {
    pub issue: Uuid,
    pub grant_operation: Uuid,
    pub upload_operation: Uuid,
    pub association_operation: Uuid,
}
pub struct VerifiedLinearAsset {
    reference: LinearAssetReference,
    comment: Comment,
    asset_url: String,
    content_digest: String,
    bytes: Vec<u8>,
    observed_at: DateTime<Utc>,
}
impl std::fmt::Debug for VerifiedLinearAsset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VerifiedLinearAsset")
            .field("reference", &self.reference)
            .field("comment_id", &self.comment.id)
            .field("content_digest", &self.content_digest)
            .field("bytes", &self.bytes.len())
            .finish_non_exhaustive()
    }
}
impl VerifiedLinearAsset {
    pub fn reference(&self) -> &LinearAssetReference {
        &self.reference
    }
    pub fn issue(&self) -> Uuid {
        self.reference.issue
    }
    pub fn comment(&self) -> &Comment {
        &self.comment
    }
    pub fn grant_operation(&self) -> Uuid {
        self.reference.grant_operation
    }
    pub fn upload_operation(&self) -> Uuid {
        self.reference.upload_operation
    }
    pub fn association_operation(&self) -> Uuid {
        self.reference.association_operation
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn digest(&self) -> &str {
        &self.content_digest
    }
    pub fn asset_url(&self) -> &str {
        &self.asset_url
    }
    pub fn content_type(&self) -> &str {
        "video/mp4"
    }
    pub fn observed_at(&self) -> DateTime<Utc> {
        self.observed_at
    }
}
pub async fn download_uploaded_asset(
    client: &LinearClient,
    host: &ControlHost,
    reference: &LinearAssetReference,
) -> Result<VerifiedLinearAsset> {
    if client.endpoint().as_str() != "https://api.linear.app/graphql" {
        return Err(Error::new(
            "asset_scope",
            "private storage authentication is supported only for the documented native Linear endpoint",
        ));
    }
    if [
        reference.issue,
        reference.grant_operation,
        reference.upload_operation,
        reference.association_operation,
    ]
    .iter()
    .any(Uuid::is_nil)
        || BTreeSet::from([
            reference.grant_operation,
            reference.upload_operation,
            reference.association_operation,
        ])
        .len()
            != 3
    {
        return Err(Error::invalid(
            "recording reference requires distinct exact owned operation IDs",
        ));
    }
    let grant = host.operation(reference.grant_operation)?;
    let upload = host.operation(reference.upload_operation)?;
    let association = host.operation(reference.association_operation)?;
    let target = format!(
        "linear:{}:{}:{}",
        client.endpoint(),
        client.workspace_id(),
        reference.issue
    );
    let runtime = host.snapshot()?.intent.runtime_id;
    for (receipt, kind) in [
        (&grant, "linear_upload_request"),
        (&upload, "linear_upload_bytes"),
        (&association, "linear_comment"),
    ] {
        if receipt.runtime_id != runtime
            || receipt.kind != kind
            || !matches!(
                receipt.phase,
                ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
            )
            || receipt.canonical_targets != [target.clone()]
            || receipt.expected.get("provider_writer").map(String::as_str)
                != Some(client.account_id())
            || receipt
                .observed_result
                .as_ref()
                .is_some_and(|v| v["effect"] == "not_sent")
        {
            return Err(Error::new(
                "asset_scope",
                "recording receipt lacks exact current account/issue acknowledgement",
            ));
        }
    }
    let required = |receipt: &Receipt, key: &str| {
        receipt.expected.get(key).cloned().ok_or_else(|| {
            Error::new(
                "asset_scope",
                "recording receipt lacks immutable content binding",
            )
        })
    };
    let url_text = required(&upload, "provider_asset_url")?;
    let url = LinearAssetUrl::parse(&url_text)?;
    let digest = required(&upload, "provider_content_digest")?;
    crate::evidence::validate_digest(&digest)?;
    let size_text = required(&upload, "provider_upload_size")?;
    let size = size_text
        .parse::<usize>()
        .map_err(|_| Error::new("asset_scope", "recording size is malformed"))?;
    if size == 0
        || size > MAX_VIDEO_BYTES
        || size_text != size.to_string()
        || required(&grant, "provider_upload_size")? != size_text
        || required(&upload, "provider_upload_grant")? != grant.operation_id.to_string()
        || required(&grant, "provider_upload_content_type")? != "video/mp4"
        || required(&grant, "provider_upload_filename")?.is_empty()
    {
        return Err(Error::new(
            "asset_scope",
            "recording grant/content type/size binding differs",
        ));
    }
    let granted = grant
        .observed_result
        .as_ref()
        .ok_or_else(|| Error::new("asset_scope", "upload grant was not acknowledged"))?;
    let uploaded = upload
        .observed_result
        .as_ref()
        .ok_or_else(|| Error::new("asset_scope", "upload bytes were not acknowledged"))?;
    if granted["asset_url"].as_str() != Some(&url_text)
        || granted["grant_operation"].as_str() != Some(&grant.operation_id.to_string())
        || grant.remote_ids.get("upload_grant_operation") != Some(&grant.operation_id.to_string())
        || uploaded["asset_url"].as_str() != Some(&url_text)
        || uploaded["content_digest"].as_str() != Some(&digest)
        || uploaded["content_type"] != "video/mp4"
        || uploaded["uploaded"] != true
    {
        return Err(Error::new(
            "asset_scope",
            "actual upload acknowledgements differ from immutable references",
        ));
    }
    client.issue(reference.issue).await?;
    let before = asset_association(client, reference.issue, &association, &url_text).await?;
    let bytes = client.read_recording(&url, size, &digest).await?;
    let after = asset_association(client, reference.issue, &association, &url_text).await?;
    if before.id != after.id
        || before.body_digest != after.body_digest
        || before.edited_at != after.edited_at
        || before.updated_at != after.updated_at
    {
        return Err(Error::new(
            "asset_scope",
            "recording association changed during readback",
        ));
    }
    Ok(VerifiedLinearAsset {
        reference: reference.clone(),
        comment: after,
        asset_url: url_text,
        content_digest: digest,
        bytes,
        observed_at: Utc::now(),
    })
}
async fn asset_association(
    client: &LinearClient,
    issue: Uuid,
    receipt: &Receipt,
    url: &str,
) -> Result<Comment> {
    let expected = receipt
        .expected
        .get("provider_body_digest")
        .ok_or_else(|| Error::new("asset_scope", "association body binding is absent"))?;
    let id = receipt
        .remote_ids
        .get("comment_id")
        .and_then(|v| Uuid::parse_str(v).ok())
        .ok_or_else(|| Error::new("asset_scope", "association comment ID is absent"))?;
    let prefix = format!("{}\n\n", linear_video_markdown(url)?);
    let comments = client.comments(issue).await?;
    let matches: Vec<_> = comments
        .into_iter()
        .filter(|c| {
            c.body_digest == *expected
                && c.user_id
                    .is_some_and(|user| user.to_string() == client.account_id())
                && !c.bot
                && !c.delegated
        })
        .collect();
    let [comment] = matches.as_slice() else {
        return Err(Error::new(
            "asset_scope",
            "recording association is missing or duplicated",
        ));
    };
    if comment.id != id
        || comment.issue_id != issue
        || comment.body_digest != bytes_digest(comment.body.as_bytes())
        || !comment.body.starts_with(&prefix)
        || comment.body[prefix.len()..].trim().is_empty()
    {
        return Err(Error::new(
            "asset_scope",
            "native recording comment is not the exact dispatched association",
        ));
    }
    Ok(comment.clone())
}
