use anki_api_proto::anki::api::v1::Note as ApiNote;
use anki_api_proto::anki::api::v1::NoteFieldUpdate;
use anki_api_proto::anki::api::v1::Notetype as ApiNotetype;
use anki_api_proto::anki::api::v1::NotetypeField;
use anki_api_proto::anki::api::v1::NotetypeTemplate;
use anki_api_proto::anki::api::v1::SortField;
use anki_proto::notes::Note as BackendNote;
use anki_proto::notetypes::Notetype as BackendNotetype;
use tonic::Status;

use crate::service::common;

pub fn map_note(note: &BackendNote, notetype: &BackendNotetype) -> Result<ApiNote, Status> {
    let ordered_fields = ordered_notetype_fields(notetype, note.fields.len())?;
    let sort_field = map_sort_field_from_ordered_fields(note, notetype, &ordered_fields);

    Ok(ApiNote {
        note_id: note.id,
        notetype_id: note.notetype_id,
        fields: ordered_fields
            .into_iter()
            .map(|field| NoteFieldUpdate {
                name: field.name,
                value: note.fields[field.ordinal].clone(),
            })
            .collect(),
        tags: note.tags.clone(),
        sort_field: Some(sort_field),
        modified_at: Some(common::timestamp_from_secs(note.mtime_secs.into())),
        usn: i64::from(note.usn),
    })
}

pub fn map_sort_field(note: &BackendNote, notetype: &BackendNotetype) -> Result<SortField, Status> {
    let ordered_fields = ordered_notetype_fields(notetype, note.fields.len())?;
    Ok(map_sort_field_from_ordered_fields(
        note,
        notetype,
        &ordered_fields,
    ))
}

fn map_sort_field_from_ordered_fields(
    note: &BackendNote,
    notetype: &BackendNotetype,
    ordered_fields: &[OrderedField],
) -> SortField {
    let sort_ordinal = notetype
        .config
        .as_ref()
        .map(|config| config.sort_field_idx as usize)
        .unwrap_or_default();
    let sort_name = ordered_fields
        .iter()
        .find(|field| field.ordinal == sort_ordinal)
        .map(|field| field.name.clone())
        .unwrap_or_default();
    let sort_value = note.fields.get(sort_ordinal).cloned().unwrap_or_default();

    SortField {
        ordinal: sort_ordinal as u32,
        name: sort_name,
        value: sort_value,
    }
}

pub fn map_notetype(notetype: &BackendNotetype) -> ApiNotetype {
    ApiNotetype {
        notetype_id: notetype.id,
        name: notetype.name.clone(),
        css: notetype
            .config
            .as_ref()
            .map(|config| config.css.clone())
            .unwrap_or_default(),
        fields: notetype
            .fields
            .iter()
            .enumerate()
            .map(|(index, field)| NotetypeField {
                ordinal: field
                    .ord
                    .as_ref()
                    .map(|ord| ord.val)
                    .unwrap_or(index as u32),
                name: field.name.clone(),
            })
            .collect(),
        templates: notetype
            .templates
            .iter()
            .enumerate()
            .map(|(index, template)| NotetypeTemplate {
                ordinal: template
                    .ord
                    .as_ref()
                    .map(|ord| ord.val)
                    .unwrap_or(index as u32),
                name: template.name.clone(),
                front: template
                    .config
                    .as_ref()
                    .map(|config| config.q_format.clone())
                    .unwrap_or_default(),
                back: template
                    .config
                    .as_ref()
                    .map(|config| config.a_format.clone())
                    .unwrap_or_default(),
            })
            .collect(),
        modified_at: Some(common::timestamp_from_secs(notetype.mtime_secs)),
        usn: i64::from(notetype.usn),
    }
}

#[derive(Clone)]
pub(crate) struct OrderedField {
    pub(crate) ordinal: usize,
    pub(crate) name: String,
}

pub(crate) fn ordered_notetype_fields(
    notetype: &BackendNotetype,
    note_field_count: usize,
) -> Result<Vec<OrderedField>, Status> {
    if notetype.fields.len() != note_field_count {
        return Err(Status::internal(format!(
            "note/notetype field mismatch for notetype_id={}: note has {} values, notetype has {} fields",
            notetype.id,
            note_field_count,
            notetype.fields.len()
        )));
    }

    let mut fields = notetype
        .fields
        .iter()
        .enumerate()
        .map(|(index, field)| OrderedField {
            ordinal: field
                .ord
                .as_ref()
                .map(|ord| ord.val as usize)
                .unwrap_or(index),
            name: field.name.clone(),
        })
        .collect::<Vec<_>>();
    fields.sort_unstable_by_key(|field| field.ordinal);

    for (expected_ordinal, field) in fields.iter().enumerate() {
        if field.ordinal != expected_ordinal {
            return Err(Status::internal(format!(
                "invalid notetype field ordinals for notetype_id={}: expected contiguous ordinals, found {} at position {}",
                notetype.id, field.ordinal, expected_ordinal
            )));
        }
    }

    Ok(fields)
}

#[cfg(test)]
mod tests {
    use anki_proto::generic::UInt32;
    use anki_proto::notes::Note;
    use anki_proto::notetypes::Notetype;
    use anki_proto::notetypes::notetype::Field;

    use super::*;

    #[test]
    fn map_note_orders_named_fields_by_ordinal() {
        let note = Note {
            id: 1,
            notetype_id: 10,
            mtime_secs: 0,
            usn: 0,
            fields: vec!["front".to_owned(), "back".to_owned()],
            ..Default::default()
        };
        let notetype = Notetype {
            id: 10,
            fields: vec![
                Field {
                    name: "Back".to_owned(),
                    ord: Some(UInt32 { val: 1 }),
                    ..Default::default()
                },
                Field {
                    name: "Front".to_owned(),
                    ord: Some(UInt32 { val: 0 }),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };

        let mapped = map_note(&note, &notetype).expect("map note");
        assert_eq!(mapped.fields.len(), 2);
        assert_eq!(mapped.fields[0].name, "Front");
        assert_eq!(mapped.fields[0].value, "front");
        assert_eq!(mapped.fields[1].name, "Back");
        assert_eq!(mapped.fields[1].value, "back");
    }

    #[test]
    fn map_note_rejects_field_count_mismatch() {
        let note = Note {
            id: 1,
            notetype_id: 10,
            mtime_secs: 0,
            usn: 0,
            fields: vec!["front".to_owned()],
            ..Default::default()
        };
        let notetype = Notetype {
            id: 10,
            fields: vec![
                Field {
                    name: "Front".to_owned(),
                    ord: Some(UInt32 { val: 0 }),
                    ..Default::default()
                },
                Field {
                    name: "Back".to_owned(),
                    ord: Some(UInt32 { val: 1 }),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };

        let err = map_note(&note, &notetype).expect_err("mismatch should fail");
        assert_eq!(err.code(), tonic::Code::Internal);
        assert!(
            err.message().contains("field mismatch"),
            "unexpected message: {}",
            err.message()
        );
    }
}
