# Update Műveletek - Részletes Implementációs Terv

## Tartalomjegyzék
1. [Áttekintés](#áttekintés)
2. [Stratégiai Döntés: In-place vs. Append-only](#stratégiai-döntés)
3. [Update Operátorok Specifikációja](#update-operátorok-specifikációja)
4. [Algoritmusok és Pszeudokód](#algoritmusok-és-pszeudokód)
5. [Adatstruktúrák](#adatstruktúrák)
6. [Implementációs Példák](#implementációs-példák)
7. [Edge Case-ek és Hibakezelés](#edge-case-ek-és-hibakezelés)
8. [Teljesítmény Megfontolások](#teljesítmény-megfontolások)

---

## Áttekintés

Az update műveletek lehetővé teszik a dokumentumok módosítását anélkül, hogy teljes replace-t kellene végezni. MongoDB-kompatibilis update operátorok implementálása.

### Célok
- ✅ MongoDB-kompatibilis update operátorok
- ✅ Atomi update műveletek
- ✅ Hatékony tárolás
- ✅ Index konzisztencia megőrzése

---

## Stratégiai Döntés: In-place vs. Append-only

### Elemzés

#### Option 1: In-place Update (Direkt módosítás)

**Előnyök:**
- ✅ Gyors write (nincs új allokáció)
- ✅ Nem növeli a fájl méretet
- ✅ Kevesebb I/O művelet

**Hátrányok:**
- ❌ Bonyolult változó méretű dokumentumok esetén
- ❌ Fragmentáció problémák
- ❌ Crash recovery nehéz
- ❌ Nincs history (rollback lehetetlen)

**Mikor használható:**
- Fix méretű dokumentumok
- Dokumentum méret NEM változik
- Egyszerű érték update-ek ($set numerikus értékre)

#### Option 2: Append-only + Tombstone (Javasolt ✅)

**Előnyök:**
- ✅ Crash-safe (write-ahead log jellegű)
- ✅ Egyszerű implementáció
- ✅ Méret változás nem probléma
- ✅ MVCC lehetőség később
- ✅ Rollback lehetséges (verziókezelés)

**Hátrányok:**
- ❌ Fájl méret növekedés
- ❌ Compaction szükséges
- ❌ Lassabb read (ha nincs index)

**Mikor használható:**
- Változó méretű dokumentumok (JSON)
- Komplex update operátorok ($push, $pull)
- MVP szinten elfogadható teljesítmény

#### Option 3: Hybrid (Későbbi optimalizáció)

**Stratégia:**
- Kis update-ek (<10% méret változás): in-place
- Nagy update-ek: append-only
- Threshold alapú döntés

### **DÖNTÉS: Append-only + Tombstone (MVP)**

**Indoklás:**
1. **Egyszerűség**: Append-only könnyebb implementálni és tesztelni
2. **Biztonság**: Crash-safe alapból, nincs corruption veszély
3. **Flexibilitás**: Később optimalizálható hybrid stratégiára
4. **Konzisztencia**: Storage engine már append-only (insert), egységes kód

**Későbbi fejlesztés:** Hybrid stratégia v0.3.0-ban, teljesítmény mérések alapján

---

## Update Operátorok Specifikációja

### 1. Field Update Operátorok

#### `$set` - Mező értékének beállítása

**Szintaxis:**
```javascript
{ $set: { <field>: <value>, ... } }
```

**Szemantika:**
- Ha a mező létezik: felülírja az értéket
- Ha a mező NEM létezik: létrehozza
- Nested field-ek támogatása: `"user.profile.age": 30`
- Tömbön belüli index: `"tags.0": "new-tag"`

**Típus változás:**
- Megengedett (JSON természete)
- Példa: `{age: "25"}` → `{age: 25}` (string → number)

**Példa:**
```python
collection.update_one(
    {"name": "János"},
    {"$set": {"age": 31, "city": "Budapest"}}
)
```

**Eredmény:**
```json
// Előtte:
{"_id": 1, "name": "János", "age": 30}

// Utána:
{"_id": 1, "name": "János", "age": 31, "city": "Budapest"}
```

---

#### `$unset` - Mező törlése

**Szintaxis:**
```javascript
{ $unset: { <field>: "", ... } }
```

**Szemantika:**
- Törli a megadott mezőt
- Az érték nem számít (konvenció szerint `""` vagy `1`)
- Ha a mező nem létezik: nincs hiba, no-op

**Példa:**
```python
collection.update_one(
    {"name": "János"},
    {"$unset": {"age": ""}}
)
```

**Eredmény:**
```json
// Előtte:
{"_id": 1, "name": "János", "age": 31, "city": "Budapest"}

// Utána:
{"_id": 1, "name": "János", "city": "Budapest"}
```

---

#### `$rename` - Mező átnevezése

**Szintaxis:**
```javascript
{ $rename: { <old_name>: <new_name>, ... } }
```

**Szemantika:**
- Átnevezi a mezőt
- Ha új név már létezik: felülírja
- Ha régi név nem létezik: no-op

**Példa:**
```python
collection.update_one(
    {"name": "János"},
    {"$rename": {"age": "years_old"}}
)
```

---

### 2. Numerikus Operátorok

#### `$inc` - Inkrementálás

**Szintaxis:**
```javascript
{ $inc: { <field>: <amount>, ... } }
```

**Szemantika:**
- Hozzáad egy numerikus értéket
- Ha mező nem létezik: létrehozza 0 értékkel, majd hozzáadja
- Ha mező NEM numerikus: hiba

**Példa:**
```python
collection.update_one(
    {"name": "János"},
    {"$inc": {"age": 1, "score": -5}}
)
```

**Eredmény:**
```json
// Előtte:
{"_id": 1, "name": "János", "age": 30, "score": 100}

// Utána:
{"_id": 1, "name": "János", "age": 31, "score": 95}
```

---

#### `$mul` - Szorzás

**Szintaxis:**
```javascript
{ $mul: { <field>: <multiplier>, ... } }
```

**Szemantika:**
- Megszorozza az értéket
- Ha mező nem létezik: létrehozza 0 értékkel
- Ha mező NEM numerikus: hiba

**Példa:**
```python
collection.update_one(
    {"name": "János"},
    {"$mul": {"price": 1.1}}  # 10% áremelés
)
```

---

#### `$min` / `$max` - Min/Max beállítás

**Szintaxis:**
```javascript
{ $min: { <field>: <value> } }
{ $max: { <field>: <value> } }
```

**Szemantika:**
- `$min`: csak akkor frissít, ha az új érték KISEBB
- `$max`: csak akkor frissít, ha az új érték NAGYOBB
- Ha mező nem létezik: beállítja az értéket

**Példa:**
```python
# Életkor maximum 65
collection.update_one(
    {"name": "János"},
    {"$max": {"age": 65}}
)
```

---

### 3. Tömb Operátorok

#### `$push` - Elem hozzáadása tömbhöz

**Szintaxis:**
```javascript
{ $push: { <field>: <value> } }
{ $push: { <field>: { $each: [<value1>, <value2>, ...] } } }
```

**Szemantika:**
- Hozzáadja az elemet a tömb végére
- Ha mező nem létezik: létrehozza a tömböt
- Ha mező NEM tömb: hiba

**Módosítók:**
- `$each`: több elem hozzáadása
- `$position`: pozíció megadása (default: végére)
- `$slice`: tömb méret limitálása
- `$sort`: rendezés beszúrás után

**Példa:**
```python
# Egyszerű push
collection.update_one(
    {"name": "János"},
    {"$push": {"tags": "mongodb"}}
)

# Több elem
collection.update_one(
    {"name": "János"},
    {"$push": {"tags": {"$each": ["rust", "python"]}}}
)

# Pozícióval és limittel
collection.update_one(
    {"name": "János"},
    {"$push": {
        "scores": {
            "$each": [95, 87],
            "$position": 0,  # Elejére
            "$slice": 5      # Max 5 elem
        }
    }}
)
```

---

#### `$pull` - Elem eltávolítása tömbből

**Szintaxis:**
```javascript
{ $pull: { <field>: <value_or_query> } }
```

**Szemantika:**
- Eltávolítja az összes egyező elemet
- Query feltétellel is működik
- Ha mező nem tömb: no-op

**Példa:**
```python
# Konkrét érték törlése
collection.update_one(
    {"name": "János"},
    {"$pull": {"tags": "old-tag"}}
)

# Query-vel
collection.update_one(
    {"name": "János"},
    {"$pull": {"scores": {"$lt": 60}}}  # Törli a 60 alatti értékeket
)
```

---

#### `$pop` - Első/utolsó elem eltávolítása

**Szintaxis:**
```javascript
{ $pop: { <field>: -1 } }  // Első elem
{ $pop: { <field>: 1 } }   // Utolsó elem
```

**Példa:**
```python
collection.update_one(
    {"name": "János"},
    {"$pop": {"tags": 1}}  # Utolsó tag törlése
)
```

---

#### `$addToSet` - Egyedi elem hozzáadása

**Szintaxis:**
```javascript
{ $addToSet: { <field>: <value> } }
{ $addToSet: { <field>: { $each: [<value1>, <value2>] } } }
```

**Szemantika:**
- Csak akkor adja hozzá, ha még nincs benne (set művelet)
- Duplikátumot nem hoz létre

**Példa:**
```python
collection.update_one(
    {"name": "János"},
    {"$addToSet": {"tags": "python"}}  # Csak egyszer lesz benne
)
```

---

### 4. Operátor Prioritás és Kombinálás

**Végrehajtási sorrend:**
1. `$rename`
2. `$unset`
3. `$set`, `$inc`, `$mul`, `$min`, `$max`
4. `$push`, `$pull`, `$pop`, `$addToSet`

**Kombinálás példa:**
```python
collection.update_one(
    {"_id": 1},
    {
        "$inc": {"age": 1},
        "$set": {"last_updated": datetime.now()},
        "$push": {"login_history": {"$each": [timestamp], "$slice": -10}}
    }
)
```

---

## Algoritmusok és Pszeudokód

### Update_One Algoritmus (Append-only stratégia)

```
FUNCTION update_one(collection_name, query, update_spec):
    // 1. Query matching - dokumentum keresése
    storage.lock_read()
    meta = storage.get_collection_meta(collection_name)

    // Szekvenciális scan (később: index használat)
    found_doc = NULL
    found_offset = NULL

    FOR offset IN meta.data_offsets:
        doc_bytes = storage.read_data(offset)
        doc = deserialize_json(doc_bytes)

        IF query_matches(doc, query):
            found_doc = doc
            found_offset = offset
            BREAK
        END IF
    END FOR

    IF found_doc == NULL:
        storage.unlock_read()
        RETURN {matched: 0, modified: 0}
    END IF

    storage.unlock_read()

    // 2. Update spec alkalmazása
    modified_doc = apply_update_spec(found_doc, update_spec)

    // 3. Változás ellenőrzése
    IF modified_doc == found_doc:  // Nincs változás
        RETURN {matched: 1, modified: 0}
    END IF

    // 4. Új verzió írása (append-only)
    storage.lock_write()

    // Új dokumentum verzió írása
    new_doc_bytes = serialize_json(modified_doc)
    new_offset = storage.write_data(new_doc_bytes)

    // 5. Tombstone létrehozása (régi verzió invalidálása)
    tombstone = {
        "_tombstone": true,
        "_id": found_doc["_id"],
        "_superseded_by": new_offset,
        "_timestamp": current_timestamp()
    }
    storage.write_tombstone(found_offset, tombstone)

    // 6. Index frissítése
    update_indexes(collection_name, found_doc, modified_doc)

    // 7. Metadata frissítése
    meta.update_count += 1
    storage.update_collection_meta(collection_name, meta)

    storage.unlock_write()

    RETURN {matched: 1, modified: 1}
END FUNCTION
```

---

### Apply Update Spec Algoritmus

```
FUNCTION apply_update_spec(document, update_spec):
    result = deep_copy(document)

    // 1. $rename operátorok
    IF "$rename" IN update_spec:
        FOR old_field, new_field IN update_spec["$rename"]:
            IF old_field IN result:
                value = result[old_field]
                DELETE result[old_field]
                result[new_field] = value
            END IF
        END FOR
    END IF

    // 2. $unset operátorok
    IF "$unset" IN update_spec:
        FOR field IN update_spec["$unset"]:
            IF field IN result:
                DELETE result[field]
            END IF
        END FOR
    END IF

    // 3. $set operátorok
    IF "$set" IN update_spec:
        FOR field, value IN update_spec["$set"]:
            set_field(result, field, value)  // Támogatja nested field-eket
        END FOR
    END IF

    // 4. $inc operátorok
    IF "$inc" IN update_spec:
        FOR field, amount IN update_spec["$inc"]:
            current = get_field(result, field, default=0)
            IF NOT is_numeric(current):
                THROW Error("Cannot increment non-numeric field")
            END IF
            set_field(result, field, current + amount)
        END FOR
    END IF

    // 5. $mul operátorok
    IF "$mul" IN update_spec:
        FOR field, multiplier IN update_spec["$mul"]:
            current = get_field(result, field, default=0)
            IF NOT is_numeric(current):
                THROW Error("Cannot multiply non-numeric field")
            END IF
            set_field(result, field, current * multiplier)
        END FOR
    END IF

    // 6. $min / $max operátorok
    IF "$min" IN update_spec:
        FOR field, value IN update_spec["$min"]:
            current = get_field(result, field, default=value)
            set_field(result, field, min(current, value))
        END FOR
    END IF

    IF "$max" IN update_spec:
        FOR field, value IN update_spec["$max"]:
            current = get_field(result, field, default=value)
            set_field(result, field, max(current, value))
        END FOR
    END IF

    // 7. $push operátorok
    IF "$push" IN update_spec:
        FOR field, push_spec IN update_spec["$push"]:
            IF push_spec IS Object AND "$each" IN push_spec:
                // Komplex push
                apply_push_complex(result, field, push_spec)
            ELSE:
                // Egyszerű push
                array = get_field(result, field, default=[])
                IF NOT is_array(array):
                    THROW Error("Cannot push to non-array field")
                END IF
                array.append(push_spec)
                set_field(result, field, array)
            END IF
        END FOR
    END IF

    // 8. $pull operátorok
    IF "$pull" IN update_spec:
        FOR field, condition IN update_spec["$pull"]:
            array = get_field(result, field, default=[])
            IF is_array(array):
                filtered = [item FOR item IN array IF NOT matches(item, condition)]
                set_field(result, field, filtered)
            END IF
        END FOR
    END IF

    // 9. $pop operátorok
    IF "$pop" IN update_spec:
        FOR field, direction IN update_spec["$pop"]:
            array = get_field(result, field, default=[])
            IF is_array(array) AND len(array) > 0:
                IF direction == -1:
                    array.remove_first()
                ELSE:
                    array.remove_last()
                END IF
                set_field(result, field, array)
            END IF
        END FOR
    END IF

    // 10. $addToSet operátorok
    IF "$addToSet" IN update_spec:
        FOR field, value_spec IN update_spec["$addToSet"]:
            array = get_field(result, field, default=[])
            IF NOT is_array(array):
                THROW Error("Cannot addToSet to non-array field")
            END IF

            IF value_spec IS Object AND "$each" IN value_spec:
                FOR item IN value_spec["$each"]:
                    IF item NOT IN array:
                        array.append(item)
                    END IF
                END FOR
            ELSE:
                IF value_spec NOT IN array:
                    array.append(value_spec)
                END IF
            END IF

            set_field(result, field, array)
        END FOR
    END IF

    RETURN result
END FUNCTION
```

---

### Update_Many Algoritmus

```
FUNCTION update_many(collection_name, query, update_spec):
    storage.lock_write()

    meta = storage.get_collection_meta(collection_name)
    matched_count = 0
    modified_count = 0

    // Összes dokumentum végigolvasása
    FOR offset IN meta.data_offsets:
        doc_bytes = storage.read_data(offset)
        doc = deserialize_json(doc_bytes)

        // Tombstone ellenőrzés
        IF doc["_tombstone"] == true:
            CONTINUE
        END IF

        // Query matching
        IF NOT query_matches(doc, query):
            CONTINUE
        END IF

        matched_count += 1

        // Update alkalmazása
        modified_doc = apply_update_spec(doc, update_spec)

        IF modified_doc == doc:
            CONTINUE  // Nincs változás
        END IF

        modified_count += 1

        // Új verzió írása
        new_doc_bytes = serialize_json(modified_doc)
        new_offset = storage.write_data(new_doc_bytes)

        // Tombstone
        tombstone = create_tombstone(doc["_id"], new_offset)
        storage.write_tombstone(offset, tombstone)

        // Index frissítés
        update_indexes(collection_name, doc, modified_doc)
    END FOR

    // Metadata frissítés
    meta.update_count += modified_count
    storage.update_collection_meta(collection_name, meta)

    storage.unlock_write()

    RETURN {matched: matched_count, modified: modified_count}
END FUNCTION
```

---

## Adatstruktúrák

### Tombstone Struktúra

```rust
#[derive(Serialize, Deserialize, Debug)]
pub struct Tombstone {
    /// Jelzi hogy ez tombstone
    pub _tombstone: bool,  // Always true

    /// Eredeti dokumentum ID
    pub _id: DocumentId,

    /// Új verzió offsetje
    pub _superseded_by: u64,

    /// Timestamp (később GC-hez)
    pub _timestamp: u64,
}
```

**Fájlban:**
```
[u32 length][JSON: {"_tombstone": true, "_id": 1, "_superseded_by": 12345, "_timestamp": 1699...}]
```

---

### UpdateSpec Struktúra

```rust
use serde_json::{Value, Map};

#[derive(Debug, Clone)]
pub struct UpdateSpec {
    pub rename: Option<Map<String, Value>>,
    pub unset: Option<Vec<String>>,
    pub set: Option<Map<String, Value>>,
    pub inc: Option<Map<String, f64>>,
    pub mul: Option<Map<String, f64>>,
    pub min: Option<Map<String, Value>>,
    pub max: Option<Map<String, Value>>,
    pub push: Option<Map<String, PushSpec>>,
    pub pull: Option<Map<String, Value>>,
    pub pop: Option<Map<String, i32>>,
    pub add_to_set: Option<Map<String, AddToSetSpec>>,
}

#[derive(Debug, Clone)]
pub enum PushSpec {
    Simple(Value),
    Complex {
        each: Vec<Value>,
        position: Option<i32>,
        slice: Option<i32>,
        sort: Option<i32>,
    },
}

#[derive(Debug, Clone)]
pub enum AddToSetSpec {
    Simple(Value),
    Each(Vec<Value>),
}
```

---

## Implementációs Példák

### Rust Implementáció - UpdateSpec Parsing

```rust
// src/update.rs
use serde_json::{Value, Map};
use crate::error::{MongoLiteError, Result};

impl UpdateSpec {
    /// Parse update spec from JSON
    pub fn from_json(update_json: &Value) -> Result<Self> {
        let obj = update_json.as_object()
            .ok_or_else(|| MongoLiteError::InvalidUpdateSpec("Update must be object".into()))?;

        let mut spec = UpdateSpec::default();

        for (op, value) in obj.iter() {
            match op.as_str() {
                "$set" => {
                    spec.set = Some(value.as_object()
                        .ok_or_else(|| MongoLiteError::InvalidUpdateSpec("$set must be object".into()))?
                        .clone());
                },
                "$unset" => {
                    let unset_obj = value.as_object()
                        .ok_or_else(|| MongoLiteError::InvalidUpdateSpec("$unset must be object".into()))?;
                    spec.unset = Some(unset_obj.keys().cloned().collect());
                },
                "$inc" => {
                    let inc_obj = value.as_object()
                        .ok_or_else(|| MongoLiteError::InvalidUpdateSpec("$inc must be object".into()))?;

                    let mut inc_map = Map::new();
                    for (field, val) in inc_obj.iter() {
                        let num = val.as_f64()
                            .ok_or_else(|| MongoLiteError::InvalidUpdateSpec(format!("$inc value must be numeric: {}", field)))?;
                        inc_map.insert(field.clone(), num);
                    }
                    spec.inc = Some(inc_map);
                },
                "$push" => {
                    spec.push = Some(Self::parse_push(value)?);
                },
                "$pull" => {
                    spec.pull = Some(value.as_object()
                        .ok_or_else(|| MongoLiteError::InvalidUpdateSpec("$pull must be object".into()))?
                        .clone());
                },
                // ... további operátorok
                _ => {
                    return Err(MongoLiteError::InvalidUpdateSpec(format!("Unknown operator: {}", op)));
                }
            }
        }

        Ok(spec)
    }

    fn parse_push(value: &Value) -> Result<Map<String, PushSpec>> {
        let obj = value.as_object()
            .ok_or_else(|| MongoLiteError::InvalidUpdateSpec("$push must be object".into()))?;

        let mut result = Map::new();

        for (field, push_value) in obj.iter() {
            if let Some(complex) = push_value.as_object() {
                if let Some(each_val) = complex.get("$each") {
                    // Complex push
                    let each_arr = each_val.as_array()
                        .ok_or_else(|| MongoLiteError::InvalidUpdateSpec("$each must be array".into()))?;

                    let position = complex.get("$position").and_then(|v| v.as_i64()).map(|v| v as i32);
                    let slice = complex.get("$slice").and_then(|v| v.as_i64()).map(|v| v as i32);
                    let sort = complex.get("$sort").and_then(|v| v.as_i64()).map(|v| v as i32);

                    result.insert(field.clone(), PushSpec::Complex {
                        each: each_arr.clone(),
                        position,
                        slice,
                        sort,
                    });
                } else {
                    result.insert(field.clone(), PushSpec::Simple(push_value.clone()));
                }
            } else {
                result.insert(field.clone(), PushSpec::Simple(push_value.clone()));
            }
        }

        Ok(result)
    }
}
```

---

### Apply Update Spec Implementáció

```rust
// src/update.rs
use serde_json::{Value, Map, Number};

pub fn apply_update_spec(mut doc: Value, spec: &UpdateSpec) -> Result<Value> {
    let doc_obj = doc.as_object_mut()
        .ok_or_else(|| MongoLiteError::InvalidDocument("Document must be object".into()))?;

    // 1. $rename
    if let Some(rename) = &spec.rename {
        for (old_field, new_field_val) in rename.iter() {
            let new_field = new_field_val.as_str()
                .ok_or_else(|| MongoLiteError::InvalidUpdateSpec("Rename target must be string".into()))?;

            if let Some(value) = doc_obj.remove(old_field) {
                doc_obj.insert(new_field.to_string(), value);
            }
        }
    }

    // 2. $unset
    if let Some(unset) = &spec.unset {
        for field in unset {
            set_nested_field(doc_obj, field, None)?;
        }
    }

    // 3. $set
    if let Some(set) = &spec.set {
        for (field, value) in set.iter() {
            set_nested_field(doc_obj, field, Some(value.clone()))?;
        }
    }

    // 4. $inc
    if let Some(inc) = &spec.inc {
        for (field, amount) in inc.iter() {
            let current = get_nested_field(doc_obj, field)
                .unwrap_or(&Value::Number(Number::from(0)));

            let current_num = current.as_f64()
                .ok_or_else(|| MongoLiteError::InvalidUpdateSpec(format!("Cannot increment non-numeric field: {}", field)))?;

            let new_value = Value::Number(Number::from_f64(current_num + amount)
                .ok_or_else(|| MongoLiteError::InvalidUpdateSpec("Invalid number".into()))?);

            set_nested_field(doc_obj, field, Some(new_value))?;
        }
    }

    // 5. $push
    if let Some(push) = &spec.push {
        for (field, push_spec) in push.iter() {
            apply_push(doc_obj, field, push_spec)?;
        }
    }

    // 6. $pull
    if let Some(pull) = &spec.pull {
        for (field, condition) in pull.iter() {
            apply_pull(doc_obj, field, condition)?;
        }
    }

    // ... további operátorok

    Ok(doc)
}

/// Helper: nested field beállítása (támogatja "user.profile.age" formát)
fn set_nested_field(obj: &mut Map<String, Value>, path: &str, value: Option<Value>) -> Result<()> {
    let parts: Vec<&str> = path.split('.').collect();

    if parts.len() == 1 {
        // Egyszerű mező
        if let Some(val) = value {
            obj.insert(path.to_string(), val);
        } else {
            obj.remove(path);
        }
        return Ok(());
    }

    // Nested mező
    let mut current = obj;
    for (i, part) in parts.iter().enumerate() {
        if i == parts.len() - 1 {
            // Utolsó elem
            if let Some(val) = value {
                current.insert(part.to_string(), val);
            } else {
                current.remove(*part);
            }
        } else {
            // Közbenső elem
            if !current.contains_key(*part) {
                current.insert(part.to_string(), Value::Object(Map::new()));
            }

            current = current.get_mut(*part)
                .and_then(|v| v.as_object_mut())
                .ok_or_else(|| MongoLiteError::InvalidUpdateSpec(format!("Path is not object: {}", part)))?;
        }
    }

    Ok(())
}

/// Helper: nested field lekérése
fn get_nested_field<'a>(obj: &'a Map<String, Value>, path: &str) -> Option<&'a Value> {
    let parts: Vec<&str> = path.split('.').collect();

    let mut current = obj;
    for (i, part) in parts.iter().enumerate() {
        if i == parts.len() - 1 {
            return current.get(*part);
        } else {
            current = current.get(*part)?.as_object()?;
        }
    }

    None
}

/// $push alkalmazása
fn apply_push(obj: &mut Map<String, Value>, field: &str, push_spec: &PushSpec) -> Result<()> {
    let mut array = match get_nested_field(obj, field) {
        Some(val) => {
            val.as_array()
                .ok_or_else(|| MongoLiteError::InvalidUpdateSpec(format!("Cannot push to non-array: {}", field)))?
                .clone()
        },
        None => Vec::new(),
    };

    match push_spec {
        PushSpec::Simple(value) => {
            array.push(value.clone());
        },
        PushSpec::Complex { each, position, slice, sort } => {
            // $each
            let pos = position.unwrap_or(array.len() as i32);
            let insert_pos = if pos < 0 {
                0
            } else {
                pos.min(array.len() as i32) as usize
            };

            for (i, item) in each.iter().enumerate() {
                array.insert(insert_pos + i, item.clone());
            }

            // $sort (ha van)
            if let Some(sort_order) = sort {
                if *sort_order == 1 {
                    array.sort_by(|a, b| compare_json_values(a, b));
                } else {
                    array.sort_by(|a, b| compare_json_values(b, a));
                }
            }

            // $slice (ha van)
            if let Some(slice_val) = slice {
                if *slice_val < 0 {
                    let start = array.len().saturating_sub((-slice_val) as usize);
                    array = array[start..].to_vec();
                } else {
                    array.truncate(*slice_val as usize);
                }
            }
        }
    }

    set_nested_field(obj, field, Some(Value::Array(array)))?;
    Ok(())
}

/// $pull alkalmazása
fn apply_pull(obj: &mut Map<String, Value>, field: &str, condition: &Value) -> Result<()> {
    let array = match get_nested_field(obj, field) {
        Some(val) => val.as_array().cloned(),
        None => return Ok(()), // Nincs mit pull-ozni
    };

    if let Some(mut arr) = array {
        // Szűrés: tartsa meg azokat amik NEM matchelnek
        arr.retain(|item| !value_matches_condition(item, condition));
        set_nested_field(obj, field, Some(Value::Array(arr)))?;
    }

    Ok(())
}

/// Érték illeszkedés ellenőrzése (egyszerű verzió)
fn value_matches_condition(value: &Value, condition: &Value) -> bool {
    // Egyszerű eset: direkt egyenlőség
    if condition.is_string() || condition.is_number() || condition.is_boolean() {
        return value == condition;
    }

    // Query object (pl. {"$gt": 5})
    if let Some(obj) = condition.as_object() {
        for (op, val) in obj.iter() {
            match op.as_str() {
                "$eq" => return value == val,
                "$ne" => return value != val,
                "$gt" => return compare_json_values(value, val) == std::cmp::Ordering::Greater,
                "$gte" => return compare_json_values(value, val) != std::cmp::Ordering::Less,
                "$lt" => return compare_json_values(value, val) == std::cmp::Ordering::Less,
                "$lte" => return compare_json_values(value, val) != std::cmp::Ordering::Greater,
                _ => return false,
            }
        }
    }

    false
}

/// JSON értékek összehasonlítása
fn compare_json_values(a: &Value, b: &Value) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    use Value::*;

    match (a, b) {
        (Number(n1), Number(n2)) => {
            n1.as_f64().partial_cmp(&n2.as_f64()).unwrap_or(Ordering::Equal)
        },
        (String(s1), String(s2)) => s1.cmp(s2),
        (Bool(b1), Bool(b2)) => b1.cmp(b2),
        _ => Ordering::Equal,
    }
}
```

---

## Edge Case-ek és Hibakezelés

### 1. Nem létező mező update-je

**Eset:**
```python
collection.update_one({"_id": 1}, {"$inc": {"counter": 1}})
# Ha "counter" nem létezik
```

**Viselkedés:**
- `$inc`, `$mul`: létrehozza 0 értékkel, majd alkalmazza
- `$set`: létrehozza a megadott értékkel
- `$unset`: no-op (nincs hiba)
- `$push`: létrehozza üres tömböt, majd push
- `$pull`: no-op

---

### 2. Típus konfliktus

**Eset:**
```python
collection.update_one({"_id": 1}, {"$inc": {"name": 1}})
# Ha "name" string
```

**Viselkedés:**
- **Hiba dobása**: `CannotIncrementNonNumeric`
- Rollback: nincs változás
- Error message: "Cannot apply $inc to non-numeric field 'name'"

---

### 3. Tömb operátor nem tömbön

**Eset:**
```python
collection.update_one({"_id": 1}, {"$push": {"age": 30}})
# Ha "age" szám
```

**Viselkedés:**
- **Hiba dobása**: `CannotPushToNonArray`
- Error message: "Cannot apply $push to non-array field 'age'"

---

### 4. Konfliktáló operátorok

**Eset:**
```python
collection.update_one({"_id": 1}, {
    "$set": {"age": 30},
    "$inc": {"age": 1}
})
```

**Viselkedés:**
- **Hiba dobása**: `ConflictingUpdateOperators`
- Error message: "Cannot update same field with multiple operators: age"

**Validáció:** Parse során ellenőrizni kell

---

### 5. Update módosítja az _id-t

**Eset:**
```python
collection.update_one({"_id": 1}, {"$set": {"_id": 999}})
```

**Viselkedés:**
- **Hiba dobása**: `CannotModifyId`
- Error message: "Cannot modify _id field"
- MongoDB kompatibilitás: _id immutable

---

### 6. Nested field konfliktus

**Eset:**
```python
collection.update_one({"_id": 1}, {
    "$set": {"user": {"name": "János"}},
    "$inc": {"user.age": 1}
})
```

**Viselkedés:**
- **Hiba dobása**: `ConflictingFieldPath`
- Error message: "Cannot update 'user' and 'user.age' simultaneously"

---

### 7. Array index out of bounds

**Eset:**
```python
collection.update_one({"_id": 1}, {"$set": {"tags.10": "new"}})
# Ha csak 3 elem van
```

**Viselkedés:**
- **Opció A (MongoDB kompatibilis):** Kitölti null-okkal
- **Opció B (MongoLite MVP):** Hiba dobása
- **Választás:** Opció B (egyszerűbb, később opt-in feature)

---

### 8. Concurrent update

**Eset:**
- Thread A és B egyszerre update-eli ugyanazt a dokumentumot

**Megoldás:**
- Write lock használata (már megvan `RwLock`)
- Szekvenciális végrehajtás
- Last-write-wins szemantika
- Később: optimistic locking (version field)

---

### 9. Update nagy dokumentumra (méret limit)

**Eset:**
- Update után dokumentum > 16MB

**Viselkedés:**
- **Hiba dobása**: `DocumentTooLarge`
- Limit: 16MB (MongoDB kompatibilitás)
- Error message: "Document size exceeds 16MB limit"

---

## Teljesítmény Megfontolások

### 1. Append-only költsége

**Probléma:**
- Minden update új dokumentum verzió
- Fájl méret gyorsan nő
- Tombstone-ok felhalmozódnak

**Megoldás:**
- Compaction (lásd IMPLEMENTATION_DELETE.md)
- Threshold: 30% tombstone arány → compaction trigger
- Background compaction (későbbi feature)

**Teljesítmény:**
- Update one: ~2-3ms (append + tombstone write)
- Fájl növekedés: ~100% per update (worst case)
- Compaction időigény: O(n) dokumentumok száma

---

### 2. Index frissítés költsége

**Probléma:**
- Update módosíthat indexelt mezőket
- Index rebuild szükséges

**Megoldás:**
- Csak érintett indexeket frissíteni
- Incremental update (nem teljes rebuild)
- B-tree insert/delete (O(log n))

**Teljesítmény:**
- Index update: +1-2ms per index
- Többszörös index: lineáris költség
- Optimization: batch index update

---

### 3. Query matching teljesítmény

**Probléma:**
- Update_one/many full collection scan
- Lassú nagy collection-ökön

**Megoldás:**
- Index használat query-hez
- Query optimizer (lásd IMPLEMENTATION_QUERY_OPTIMIZER.md)
- Index hint lehetőség

**Teljesítmény:**
- Full scan: O(n) dokumentumok
- Index használat: O(log n) + O(m) matches
- 10K dokumentum: ~50ms scan vs. ~2ms index

---

### 4. Memory használat

**Probléma:**
- Teljes dokumentum memóriában (deserialize, modify, serialize)
- Nagy dokumentumok esetén sok memória

**Megoldás:**
- Streaming JSON parsing (későbbi optimalizáció)
- Partial update (csak módosított mezők)
- Memory limit per transaction

**Jelenlegi:**
- ~2x dokumentum méret memória (eredeti + modified)
- 1MB dokumentum → ~2MB RAM
- Elfogadható MVP szinten

---

### 5. Batch update optimalizálás

**Javaslat:**
- Update_many: batch commit
- Metadata csak végén frissül
- Index rebuild egy lépésben

**Implementáció (későbbi):**
```rust
fn update_many_batched(query, update_spec, batch_size: usize) {
    let mut batch = Vec::new();

    for doc in matching_documents {
        batch.push(modified_doc);

        if batch.len() >= batch_size {
            flush_batch(&batch);
            batch.clear();
        }
    }

    flush_batch(&batch); // Remaining
}
```

---

## Tesztelési Stratégia

### Unit tesztek

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_simple_field() {
        let doc = json!({"_id": 1, "name": "János", "age": 30});
        let spec = UpdateSpec {
            set: Some(Map::from([("age".to_string(), json!(31))])),
            ..Default::default()
        };

        let result = apply_update_spec(doc, &spec).unwrap();
        assert_eq!(result["age"], 31);
    }

    #[test]
    fn test_inc_nonexistent_field() {
        let doc = json!({"_id": 1, "name": "János"});
        let spec = UpdateSpec {
            inc: Some(Map::from([("counter".to_string(), 5.0)])),
            ..Default::default()
        };

        let result = apply_update_spec(doc, &spec).unwrap();
        assert_eq!(result["counter"], 5);
    }

    #[test]
    fn test_push_to_array() {
        let doc = json!({"_id": 1, "tags": ["a", "b"]});
        let spec = UpdateSpec {
            push: Some(Map::from([("tags".to_string(), PushSpec::Simple(json!("c")))])),
            ..Default::default()
        };

        let result = apply_update_spec(doc, &spec).unwrap();
        assert_eq!(result["tags"], json!(["a", "b", "c"]));
    }

    #[test]
    fn test_inc_non_numeric_error() {
        let doc = json!({"_id": 1, "name": "János"});
        let spec = UpdateSpec {
            inc: Some(Map::from([("name".to_string(), 1.0)])),
            ..Default::default()
        };

        let result = apply_update_spec(doc, &spec);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), MongoLiteError::InvalidUpdateSpec(_)));
    }

    #[test]
    fn test_pull_with_condition() {
        let doc = json!({"_id": 1, "scores": [50, 70, 90, 40]});
        let spec = UpdateSpec {
            pull: Some(Map::from([("scores".to_string(), json!({"$lt": 60}))])),
            ..Default::default()
        };

        let result = apply_update_spec(doc, &spec).unwrap();
        assert_eq!(result["scores"], json!([70, 90]));
    }

    #[test]
    fn test_multiple_operators() {
        let doc = json!({"_id": 1, "age": 30, "score": 100, "tags": ["a"]});
        let spec = UpdateSpec {
            inc: Some(Map::from([("age".to_string(), 1.0)])),
            set: Some(Map::from([("updated".to_string(), json!(true))])),
            push: Some(Map::from([("tags".to_string(), PushSpec::Simple(json!("b")))])),
            ..Default::default()
        };

        let result = apply_update_spec(doc, &spec).unwrap();
        assert_eq!(result["age"], 31);
        assert_eq!(result["updated"], true);
        assert_eq!(result["tags"], json!(["a", "b"]));
    }
}
```

---

## Roadmap és Prioritás

### MVP (v0.2.0) - 2-3 hét
- ✅ `$set` operátor
- ✅ `$inc` operátor
- ✅ `$unset` operátor
- ✅ `update_one()` append-only implementáció
- ✅ Tombstone kezelés
- ✅ Alapvető error handling

### v0.2.1 - 1 hét
- ✅ `$push` (egyszerű)
- ✅ `$pull` (egyszerű)
- ✅ `update_many()`
- ✅ Nested field támogatás

### v0.3.0 - 2-3 hét
- ✅ `$push` komplex ($each, $position, $slice, $sort)
- ✅ `$mul`, `$min`, `$max`
- ✅ `$addToSet`
- ✅ `$pop`
- ✅ `$rename`
- ✅ Index frissítés integráció
- ✅ Compaction trigger

### v0.4.0 - Optimalizáció
- Hybrid update stratégia (in-place kis változásokra)
- Batch update optimalizálás
- Streaming JSON parsing
- Performance benchmarking

---

## Összefoglalás

### Kulcs Döntések

1. **Append-only + Tombstone stratégia** (MVP)
   - Egyszerű, crash-safe
   - Kompakció szükséges
   - Későbbi hybrid optimalizálás

2. **Teljes MongoDB operátor támogatás**
   - Field: $set, $unset, $rename
   - Numeric: $inc, $mul, $min, $max
   - Array: $push, $pull, $pop, $addToSet

3. **Nested field támogatás**
   - "user.profile.age" szintaxis
   - Automatikus object létrehozás

4. **Strict validáció**
   - Típus ellenőrzés
   - Konfliktus detektálás
   - _id immutability

### Implementációs Sorrend

1. **Hét 1:** UpdateSpec parsing + apply_update_spec
2. **Hét 2:** update_one implementáció + tombstone
3. **Hét 3:** update_many + tesztelés
4. **Hét 4:** Array operátorok + edge case-ek

### Sikerkritériumok

- ✅ MongoDB-kompatibilis API
- ✅ Atomi update műveletek
- ✅ Index konzisztencia
- ✅ < 5ms update latency (1000 dokumentumos collection-ön)
- ✅ Crash recovery (append-only miatt)

---

**Következő lépés:** `IMPLEMENTATION_DELETE.md` - Delete és compaction mechanizmus
