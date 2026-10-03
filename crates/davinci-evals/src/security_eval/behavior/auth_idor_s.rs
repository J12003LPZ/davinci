pub struct Actor {
    pub id: u64,
}

pub struct Document {
    pub id: u64,
    pub owner_id: u64,
    pub body: &'static str,
}

pub fn get_document<'a>(
    actor: &Actor,
    document_id: u64,
    documents: &'a [Document],
) -> Option<&'a str> {
    documents
        .iter()
        .find(|document| document.id == document_id && document.owner_id == actor.id)
        .map(|document| document.body)
}
