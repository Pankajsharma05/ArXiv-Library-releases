use crate::arxiv::Paper;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Collection {
    pub id: String,
    pub name: String,
    pub parent_id: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub archived: bool,
}

#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
}

/// ~/.local/share/ArxivLibrary (Linux) or ~/Library/Application Support/ArxivLibrary (macOS)
pub fn data_dir() -> PathBuf {
    let mut dir = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    dir.push("ArxivLibrary");
    std::fs::create_dir_all(&dir).ok();
    dir
}

impl Db {
    pub fn open() -> Result<Self, String> {
        let path = data_dir().join("library.sqlite");
        let conn = Connection::open(&path).map_err(|e| e.to_string())?;
        let _ = conn.execute_batch("PRAGMA foreign_keys = ON;");
        let db = Db { conn: Arc::new(Mutex::new(conn)) };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS paper (
                arxiv_id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                summary TEXT NOT NULL,
                authors TEXT NOT NULL,
                categories TEXT NOT NULL,
                primary_category TEXT NOT NULL,
                published TEXT NOT NULL,
                updated TEXT NOT NULL,
                pdf_url TEXT NOT NULL,
                abs_url TEXT NOT NULL,
                doi TEXT,
                journal_ref TEXT,
                local_pdf_path TEXT,
                note TEXT NOT NULL DEFAULT '',
                added_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS collection (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                parent_id TEXT,
                created_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS paper_collection (
                paper_id TEXT NOT NULL REFERENCES paper(arxiv_id) ON DELETE CASCADE,
                collection_id TEXT NOT NULL REFERENCES collection(id) ON DELETE CASCADE,
                PRIMARY KEY (paper_id, collection_id)
            );
            CREATE TABLE IF NOT EXISTS edge (
                id TEXT PRIMARY KEY,
                source TEXT NOT NULL REFERENCES paper(arxiv_id) ON DELETE CASCADE,
                target TEXT NOT NULL REFERENCES paper(arxiv_id) ON DELETE CASCADE,
                label TEXT NOT NULL DEFAULT '',
                direction TEXT NOT NULL DEFAULT 'none',
                created_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS tag (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                color TEXT
            );
            CREATE TABLE IF NOT EXISTS paper_tag (
                paper_id TEXT NOT NULL REFERENCES paper(arxiv_id) ON DELETE CASCADE,
                tag_id TEXT NOT NULL REFERENCES tag(id) ON DELETE CASCADE,
                PRIMARY KEY (paper_id, tag_id)
            );
            CREATE TABLE IF NOT EXISTS view_history (
                arxiv_id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                authors TEXT NOT NULL,
                primary_category TEXT,
                published TEXT,
                abs_url TEXT,
                viewed_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS bib_entry (
                id TEXT PRIMARY KEY,
                doi TEXT,
                raw_json TEXT NOT NULL,
                added_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS bib_folder (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                color TEXT
            );
            CREATE TABLE IF NOT EXISTS bib_entry_folder (
                bib_id TEXT NOT NULL,
                folder_id TEXT NOT NULL,
                PRIMARY KEY (bib_id, folder_id)
            );
            CREATE TABLE IF NOT EXISTS bib_tag (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                color TEXT
            );
            CREATE TABLE IF NOT EXISTS bib_entry_tag (
                bib_id TEXT NOT NULL,
                tag_id TEXT NOT NULL,
                PRIMARY KEY (bib_id, tag_id)
            );
            ",
        )
        .map_err(|e| e.to_string())?;

        // Add columns for databases created before doi/journal_ref existed.
        // These error harmlessly if the column is already present, so ignore failures.
        let _ = conn.execute("ALTER TABLE paper ADD COLUMN doi TEXT", []);
        let _ = conn.execute("ALTER TABLE paper ADD COLUMN journal_ref TEXT", []);
        let _ = conn.execute("ALTER TABLE collection ADD COLUMN color TEXT", []);
        let _ = conn.execute("ALTER TABLE collection ADD COLUMN archived INTEGER NOT NULL DEFAULT 0", []);
        let _ = conn.execute("ALTER TABLE paper ADD COLUMN reading_status TEXT NOT NULL DEFAULT 'unread'", []);
        let _ = conn.execute("ALTER TABLE paper ADD COLUMN last_opened TEXT", []);
        let _ = conn.execute("ALTER TABLE paper ADD COLUMN trashed INTEGER NOT NULL DEFAULT 0", []);
        // Manual ordering for the sidebar lists. Default large so existing rows
        // sort after any newly-ordered ones until the user drags them.
        let _ = conn.execute("ALTER TABLE collection ADD COLUMN sort_order INTEGER NOT NULL DEFAULT 1000000", []);
        let _ = conn.execute("ALTER TABLE tag ADD COLUMN sort_order INTEGER NOT NULL DEFAULT 1000000", []);
        let _ = conn.execute("ALTER TABLE paper ADD COLUMN trashed_at TEXT", []);

        // One-time cleanup of orphans left by older buggy deletes:
        // (a) collections whose parent_id points to a now-deleted collection,
        // (b) memberships referencing collections or papers that no longer exist.
        // Loop a few times so multi-level orphan chains are fully removed.
        for _ in 0..10 {
            let removed = conn.execute(
                "DELETE FROM collection WHERE parent_id IS NOT NULL
                 AND parent_id NOT IN (SELECT id FROM collection)",
                [],
            ).unwrap_or(0);
            if removed == 0 { break; }
        }
        let _ = conn.execute(
            "DELETE FROM paper_collection WHERE collection_id NOT IN (SELECT id FROM collection)", []);
        let _ = conn.execute(
            "DELETE FROM paper_collection WHERE paper_id NOT IN (SELECT arxiv_id FROM paper)", []);
        Ok(())
    }

    pub fn save_paper(&self, p: &Paper) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        // IMPORTANT: use an UPSERT (ON CONFLICT DO UPDATE), NOT INSERT OR REPLACE.
        // INSERT OR REPLACE deletes the existing row first, which fires the
        // ON DELETE CASCADE on paper_collection / paper_tag / edge and silently
        // wipes the paper's collections, tags, and graph edges. An UPSERT updates
        // the row in place, leaving those relationships intact.
        conn.execute(
            "INSERT INTO paper
             (arxiv_id,title,summary,authors,categories,primary_category,
              published,updated,pdf_url,abs_url,doi,journal_ref,local_pdf_path,note,added_at,
              reading_status,last_opened)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,'unread',NULL)
             ON CONFLICT(arxiv_id) DO UPDATE SET
               title=excluded.title,
               summary=excluded.summary,
               authors=excluded.authors,
               categories=excluded.categories,
               primary_category=excluded.primary_category,
               published=excluded.published,
               updated=excluded.updated,
               pdf_url=excluded.pdf_url,
               abs_url=excluded.abs_url,
               doi=excluded.doi,
               journal_ref=excluded.journal_ref,
               local_pdf_path=COALESCE(excluded.local_pdf_path, paper.local_pdf_path),
               note=excluded.note",
            params![
                p.arxiv_id, p.title, p.summary,
                serde_json::to_string(&p.authors).unwrap(),
                serde_json::to_string(&p.categories).unwrap(),
                p.primary_category, p.published, p.updated,
                p.pdf_url, p.abs_url, p.doi, p.journal_ref, p.local_pdf_path, p.note,
                chrono::Utc::now().to_rfc3339(),
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn all_papers(&self) -> Result<Vec<Paper>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT arxiv_id,title,summary,authors,categories,primary_category,published,updated,pdf_url,abs_url,local_pdf_path,note,doi,journal_ref,reading_status,last_opened,trashed FROM paper ORDER BY added_at DESC")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok(Paper {
                    arxiv_id: r.get(0)?,
                    title: r.get(1)?,
                    summary: r.get(2)?,
                    authors: serde_json::from_str(&r.get::<_, String>(3)?).unwrap_or_default(),
                    categories: serde_json::from_str(&r.get::<_, String>(4)?).unwrap_or_default(),
                    primary_category: r.get(5)?,
                    published: r.get(6)?,
                    updated: r.get(7)?,
                    pdf_url: r.get(8)?,
                    abs_url: r.get(9)?,
                    local_pdf_path: r.get(10)?,
                    note: r.get(11)?,
                    doi: r.get(12)?,
                    journal_ref: r.get(13)?,
                    reading_status: r.get::<_, Option<String>>(14)?.unwrap_or_else(|| "unread".into()),
                    last_opened: r.get(15)?,
                    trashed: r.get::<_, Option<i64>>(16)?.unwrap_or(0) != 0,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }

    pub fn set_trashed(&self, id: &str, trashed: bool) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        let at = if trashed { Some(chrono::Utc::now().to_rfc3339()) } else { None };
        conn.execute("UPDATE paper SET trashed=?1, trashed_at=?2 WHERE arxiv_id=?3",
            params![trashed as i64, at, id]).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Permanently delete all trashed papers; returns their local PDF paths for cleanup.
    pub fn empty_trash(&self) -> Result<Vec<String>, String> {
        let conn = self.conn.lock().unwrap();
        let paths: Vec<String> = {
            let mut stmt = conn.prepare("SELECT local_pdf_path FROM paper WHERE trashed=1 AND local_pdf_path IS NOT NULL")
                .map_err(|e| e.to_string())?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0)).map_err(|e| e.to_string())?;
            rows.filter_map(|x| x.ok()).collect()
        };
        conn.execute("DELETE FROM paper WHERE trashed=1", []).map_err(|e| e.to_string())?;
        Ok(paths)
    }

    pub fn set_reading_status(&self, id: &str, status: &str) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute("UPDATE paper SET reading_status=?1 WHERE arxiv_id=?2", params![status, id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn mark_opened(&self, id: &str) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute("UPDATE paper SET last_opened=?1 WHERE arxiv_id=?2",
            params![chrono::Utc::now().to_rfc3339(), id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn delete_paper(&self, id: &str) -> Result<Option<String>, String> {
        let conn = self.conn.lock().unwrap();
        let local: Option<String> = conn
            .query_row("SELECT local_pdf_path FROM paper WHERE arxiv_id=?1", params![id], |r| r.get(0))
            .ok()
            .flatten();
        conn.execute("DELETE FROM paper WHERE arxiv_id=?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(local)
    }

    pub fn update_note(&self, id: &str, note: &str) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute("UPDATE paper SET note=?1 WHERE arxiv_id=?2", params![note, id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn set_local_path(&self, id: &str, path: &str) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute("UPDATE paper SET local_pdf_path=?1 WHERE arxiv_id=?2", params![path, id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Clears the local PDF reference for one paper (keeps the library entry).
    pub fn clear_local_path(&self, id: &str) -> Result<Option<String>, String> {
        let conn = self.conn.lock().unwrap();
        let local: Option<String> = conn
            .query_row("SELECT local_pdf_path FROM paper WHERE arxiv_id=?1", params![id], |r| r.get(0))
            .ok()
            .flatten();
        conn.execute("UPDATE paper SET local_pdf_path=NULL WHERE arxiv_id=?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(local)
    }

    /// Returns (arxiv_id, local_pdf_path) for all papers that have a downloaded PDF.
    pub fn papers_with_pdfs(&self) -> Result<Vec<(String, String)>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT arxiv_id, local_pdf_path FROM paper WHERE local_pdf_path IS NOT NULL")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }

    // Collections

    pub fn add_collection(&self, name: &str, parent_id: Option<String>) -> Result<Collection, String> {
        let id = new_id();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO collection (id,name,parent_id,created_at) VALUES (?1,?2,?3,?4)",
            params![id, name, parent_id, chrono::Utc::now().to_rfc3339()],
        )
        .map_err(|e| e.to_string())?;
        Ok(Collection { id, name: name.to_string(), parent_id, color: None, archived: false })
    }

    pub fn rename_collection(&self, id: &str, name: &str) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute("UPDATE collection SET name=?1 WHERE id=?2", params![name, id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn set_collection_color(&self, id: &str, color: Option<String>) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute("UPDATE collection SET color=?1 WHERE id=?2", params![color, id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn reorder_collections(&self, ids: Vec<String>) -> Result<(), String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        for (i, id) in ids.iter().enumerate() {
            tx.execute("UPDATE collection SET sort_order=?1 WHERE id=?2", params![i as i64, id])
                .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn set_collection_archived(&self, id: &str, archived: bool) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute("UPDATE collection SET archived=?1 WHERE id=?2", params![archived as i64, id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn delete_collection(&self, id: &str) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        // Gather this collection and all descendants (FKs may be off, so do it manually).
        let mut to_delete = vec![id.to_string()];
        let mut frontier = vec![id.to_string()];
        while let Some(parent) = frontier.pop() {
            let children: Vec<String> = {
                let mut stmt = conn.prepare("SELECT id FROM collection WHERE parent_id=?1")
                    .map_err(|e| e.to_string())?;
                let rows = stmt.query_map(params![parent], |r| r.get::<_, String>(0))
                    .map_err(|e| e.to_string())?;
                rows.filter_map(|x| x.ok()).collect()
            };
            for c in children {
                to_delete.push(c.clone());
                frontier.push(c);
            }
        }
        for cid in &to_delete {
            conn.execute("DELETE FROM paper_collection WHERE collection_id=?1", params![cid])
                .map_err(|e| e.to_string())?;
            conn.execute("DELETE FROM collection WHERE id=?1", params![cid])
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn all_collections(&self) -> Result<Vec<Collection>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT id,name,parent_id,color,archived FROM collection ORDER BY sort_order ASC, name COLLATE NOCASE")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok(Collection { id: r.get(0)?, name: r.get(1)?, parent_id: r.get(2)?, color: r.get(3)?,
                    archived: r.get::<_, Option<i64>>(4)?.unwrap_or(0) != 0 })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }

    pub fn assign(&self, paper_id: &str, collection_id: &str) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO paper_collection (paper_id,collection_id) VALUES (?1,?2)",
            params![paper_id, collection_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn unassign(&self, paper_id: &str, collection_id: &str) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM paper_collection WHERE paper_id=?1 AND collection_id=?2",
            params![paper_id, collection_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Returns (paper_id, collection_id) pairs for building membership on the frontend.
    pub fn all_membership(&self) -> Result<Vec<(String, String)>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT paper_id, collection_id FROM paper_collection")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }

    // Edges (paper-to-paper connections for the graph)

    pub fn add_edge(&self, source: &str, target: &str, label: &str, direction: &str) -> Result<Edge, String> {
        let id = new_id();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO edge (id,source,target,label,direction,created_at) VALUES (?1,?2,?3,?4,?5,?6)",
            params![id, source, target, label, direction, chrono::Utc::now().to_rfc3339()],
        )
        .map_err(|e| e.to_string())?;
        Ok(Edge { id, source: source.into(), target: target.into(), label: label.into(), direction: direction.into() })
    }

    pub fn update_edge(&self, id: &str, label: &str, direction: &str) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute("UPDATE edge SET label=?1, direction=?2 WHERE id=?3", params![label, direction, id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn delete_edge(&self, id: &str) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM edge WHERE id=?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn all_edges(&self) -> Result<Vec<Edge>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT id,source,target,label,direction FROM edge")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok(Edge { id: r.get(0)?, source: r.get(1)?, target: r.get(2)?, label: r.get(3)?, direction: r.get(4)? })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }

    // ---- View history ----

    pub fn record_view(&self, p: &HistoryEntry) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO view_history (arxiv_id,title,authors,primary_category,published,abs_url,viewed_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![p.arxiv_id, p.title, serde_json::to_string(&p.authors).unwrap_or_default(),
                p.primary_category, p.published, p.abs_url, chrono::Utc::now().to_rfc3339()],
        ).map_err(|e| e.to_string())?;
        // Keep only the 50 most recent.
        conn.execute(
            "DELETE FROM view_history WHERE arxiv_id NOT IN
             (SELECT arxiv_id FROM view_history ORDER BY viewed_at DESC LIMIT 50)",
            [],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn all_history(&self) -> Result<Vec<HistoryEntry>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT arxiv_id,title,authors,primary_category,published,abs_url,viewed_at
             FROM view_history ORDER BY viewed_at DESC").map_err(|e| e.to_string())?;
        let rows = stmt.query_map([], |r| {
            Ok(HistoryEntry {
                arxiv_id: r.get(0)?,
                title: r.get(1)?,
                authors: serde_json::from_str(&r.get::<_, String>(2)?).unwrap_or_default(),
                primary_category: r.get(3)?,
                published: r.get(4)?,
                abs_url: r.get(5)?,
                viewed_at: r.get(6)?,
            })
        }).map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }

    pub fn clear_history(&self) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM view_history", []).map_err(|e| e.to_string())?;
        Ok(())
    }

    // ---- Bibliography ----

    pub fn add_bib_entry(&self, doi: Option<String>, raw_json: &str) -> Result<String, String> {
        let id = new_id();
        let conn = self.conn.lock().unwrap();
        conn.execute("INSERT INTO bib_entry (id,doi,raw_json,added_at) VALUES (?1,?2,?3,?4)",
            params![id, doi, raw_json, chrono::Utc::now().to_rfc3339()]).map_err(|e| e.to_string())?;
        Ok(id)
    }

    pub fn delete_bib_entry(&self, id: &str) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM bib_entry WHERE id=?1", params![id]).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn all_bib_entries(&self) -> Result<Vec<BibEntry>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT id,doi,raw_json FROM bib_entry ORDER BY added_at DESC")
            .map_err(|e| e.to_string())?;
        let mut entries: Vec<BibEntry> = stmt.query_map([], |r| {
            Ok(BibEntry { id: r.get(0)?, doi: r.get(1)?, raw_json: r.get(2)?, folder_ids: vec![], tag_ids: vec![] })
        }).map_err(|e| e.to_string())?.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
        for e in &mut entries {
            let mut fs = conn.prepare("SELECT folder_id FROM bib_entry_folder WHERE bib_id=?1").map_err(|x| x.to_string())?;
            e.folder_ids = fs.query_map(params![e.id], |r| r.get::<_, String>(0)).map_err(|x| x.to_string())?
                .filter_map(|x| x.ok()).collect();
            let mut ts = conn.prepare("SELECT tag_id FROM bib_entry_tag WHERE bib_id=?1").map_err(|x| x.to_string())?;
            e.tag_ids = ts.query_map(params![e.id], |r| r.get::<_, String>(0)).map_err(|x| x.to_string())?
                .filter_map(|x| x.ok()).collect();
        }
        Ok(entries)
    }

    // ---- Bib folders ----
    pub fn add_bib_folder(&self, name: &str) -> Result<BibFolder, String> {
        let id = new_id();
        let conn = self.conn.lock().unwrap();
        conn.execute("INSERT INTO bib_folder (id,name,color) VALUES (?1,?2,NULL)", params![id, name])
            .map_err(|e| e.to_string())?;
        Ok(BibFolder { id, name: name.to_string(), color: None })
    }
    pub fn rename_bib_folder(&self, id: &str, name: &str) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute("UPDATE bib_folder SET name=?1 WHERE id=?2", params![name, id]).map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn set_bib_folder_color(&self, id: &str, color: Option<String>) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute("UPDATE bib_folder SET color=?1 WHERE id=?2", params![color, id]).map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn delete_bib_folder(&self, id: &str) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM bib_entry_folder WHERE folder_id=?1", params![id]).map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM bib_folder WHERE id=?1", params![id]).map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn all_bib_folders(&self) -> Result<Vec<BibFolder>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT id,name,color FROM bib_folder ORDER BY name COLLATE NOCASE")
            .map_err(|e| e.to_string())?;
        let rows = stmt.query_map([], |r| Ok(BibFolder { id: r.get(0)?, name: r.get(1)?, color: r.get(2)? }))
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }
    pub fn assign_bib_folder(&self, bib_id: &str, folder_id: &str) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute("INSERT OR IGNORE INTO bib_entry_folder (bib_id,folder_id) VALUES (?1,?2)", params![bib_id, folder_id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn unassign_bib_folder(&self, bib_id: &str, folder_id: &str) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM bib_entry_folder WHERE bib_id=?1 AND folder_id=?2", params![bib_id, folder_id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    // ---- Bib tags ----
    pub fn add_bib_tag(&self, name: &str, color: Option<String>) -> Result<BibTag, String> {
        let conn = self.conn.lock().unwrap();
        // Reuse existing tag of the same name if present.
        if let Ok(id) = conn.query_row("SELECT id FROM bib_tag WHERE name=?1", params![name], |r| r.get::<_, String>(0)) {
            return Ok(BibTag { id, name: name.to_string(), color });
        }
        let id = new_id();
        conn.execute("INSERT INTO bib_tag (id,name,color) VALUES (?1,?2,?3)", params![id, name, color])
            .map_err(|e| e.to_string())?;
        Ok(BibTag { id, name: name.to_string(), color })
    }
    pub fn delete_bib_tag(&self, id: &str) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM bib_entry_tag WHERE tag_id=?1", params![id]).map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM bib_tag WHERE id=?1", params![id]).map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn all_bib_tags(&self) -> Result<Vec<BibTag>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT id,name,color FROM bib_tag ORDER BY name COLLATE NOCASE")
            .map_err(|e| e.to_string())?;
        let rows = stmt.query_map([], |r| Ok(BibTag { id: r.get(0)?, name: r.get(1)?, color: r.get(2)? }))
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }
    pub fn assign_bib_tag(&self, bib_id: &str, tag_id: &str) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute("INSERT OR IGNORE INTO bib_entry_tag (bib_id,tag_id) VALUES (?1,?2)", params![bib_id, tag_id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn unassign_bib_tag(&self, bib_id: &str, tag_id: &str) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM bib_entry_tag WHERE bib_id=?1 AND tag_id=?2", params![bib_id, tag_id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub arxiv_id: String,
    pub title: String,
    pub authors: Vec<String>,
    pub primary_category: Option<String>,
    pub published: Option<String>,
    pub abs_url: Option<String>,
    #[serde(default)]
    pub viewed_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BibEntry {
    pub id: String,
    pub doi: Option<String>,
    pub raw_json: String,
    #[serde(default)]
    pub folder_ids: Vec<String>,
    #[serde(default)]
    pub tag_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BibFolder {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub color: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BibTag {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub color: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub id: String,
    pub source: String,
    pub target: String,
    pub label: String,
    /// "none" (A—B), "forward" (A→B), or "both" (A↔B)
    pub direction: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tag {
    pub id: String,
    pub name: String,
    pub color: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Backup {
    pub version: u32,
    pub papers: Vec<Paper>,
    pub collections: Vec<Collection>,
    pub membership: Vec<(String, String)>,
    pub edges: Vec<Edge>,
    pub tags: Vec<Tag>,
    pub paper_tags: Vec<(String, String)>,
}

impl Db {
    pub fn add_tag(&self, name: &str, color: Option<String>) -> Result<Tag, String> {
        let conn = self.conn.lock().unwrap();
        // Reuse an existing tag of the same name if present.
        if let Ok(existing) = conn.query_row(
            "SELECT id,name,color FROM tag WHERE name=?1", params![name],
            |r| Ok(Tag { id: r.get(0)?, name: r.get(1)?, color: r.get(2)? })) {
            return Ok(existing);
        }
        let id = new_id();
        conn.execute("INSERT INTO tag (id,name,color) VALUES (?1,?2,?3)",
            params![id, name, color]).map_err(|e| e.to_string())?;
        Ok(Tag { id, name: name.into(), color })
    }

    pub fn delete_tag(&self, id: &str) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM tag WHERE id=?1", params![id]).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn set_tag_color(&self, id: &str, color: Option<String>) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute("UPDATE tag SET color=?1 WHERE id=?2", params![color, id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn reorder_tags(&self, ids: Vec<String>) -> Result<(), String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        for (i, id) in ids.iter().enumerate() {
            tx.execute("UPDATE tag SET sort_order=?1 WHERE id=?2", params![i as i64, id])
                .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn all_tags(&self) -> Result<Vec<Tag>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT id,name,color FROM tag ORDER BY sort_order ASC, name COLLATE NOCASE")
            .map_err(|e| e.to_string())?;
        let rows = stmt.query_map([], |r| Ok(Tag { id: r.get(0)?, name: r.get(1)?, color: r.get(2)? }))
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }

    pub fn tag_paper(&self, paper_id: &str, tag_id: &str) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute("INSERT OR IGNORE INTO paper_tag (paper_id,tag_id) VALUES (?1,?2)",
            params![paper_id, tag_id]).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn untag_paper(&self, paper_id: &str, tag_id: &str) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM paper_tag WHERE paper_id=?1 AND tag_id=?2",
            params![paper_id, tag_id]).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn all_paper_tags(&self) -> Result<Vec<(String, String)>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT paper_id, tag_id FROM paper_tag")
            .map_err(|e| e.to_string())?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }

    pub fn export_backup(&self) -> Result<Backup, String> {
        Ok(Backup {
            version: 1,
            papers: self.all_papers()?,
            collections: self.all_collections()?,
            membership: self.all_membership()?,
            edges: self.all_edges()?,
            tags: self.all_tags()?,
            paper_tags: self.all_paper_tags()?,
        })
    }

    /// Imports a backup, merging into the current library (INSERT OR IGNORE/REPLACE).
    pub fn import_backup(&self, b: &Backup) -> Result<(), String> {
        for p in &b.papers { self.save_paper(p)?; }
        // Restore reading status/last_opened which save_paper preserves only if row pre-exists.
        {
            let conn = self.conn.lock().unwrap();
            for p in &b.papers {
                let _ = conn.execute("UPDATE paper SET reading_status=?1, last_opened=?2 WHERE arxiv_id=?3",
                    params![p.reading_status, p.last_opened, p.arxiv_id]);
            }
            for c in &b.collections {
                let _ = conn.execute(
                    "INSERT OR REPLACE INTO collection (id,name,parent_id,color,created_at)
                     VALUES (?1,?2,?3,?4, COALESCE((SELECT created_at FROM collection WHERE id=?1), ?5))",
                    params![c.id, c.name, c.parent_id, c.color, chrono::Utc::now().to_rfc3339()]);
            }
            for (pid, cid) in &b.membership {
                let _ = conn.execute("INSERT OR IGNORE INTO paper_collection (paper_id,collection_id) VALUES (?1,?2)",
                    params![pid, cid]);
            }
            for e in &b.edges {
                let _ = conn.execute(
                    "INSERT OR REPLACE INTO edge (id,source,target,label,direction,created_at)
                     VALUES (?1,?2,?3,?4,?5, COALESCE((SELECT created_at FROM edge WHERE id=?1), ?6))",
                    params![e.id, e.source, e.target, e.label, e.direction, chrono::Utc::now().to_rfc3339()]);
            }
            for t in &b.tags {
                let _ = conn.execute("INSERT OR REPLACE INTO tag (id,name,color) VALUES (?1,?2,?3)",
                    params![t.id, t.name, t.color]);
            }
            for (pid, tid) in &b.paper_tags {
                let _ = conn.execute("INSERT OR IGNORE INTO paper_tag (paper_id,tag_id) VALUES (?1,?2)",
                    params![pid, tid]);
            }
        }
        Ok(())
    }
}

fn new_id() -> String {
    // Lightweight unique id without pulling in the uuid crate.
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    format!("c{nanos:x}")
}
