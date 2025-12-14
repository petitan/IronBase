#!/usr/bin/env python3
"""
Script to populate IronBase with Hungarian folk tales from MEK.
Creates a nested collection structure for fulltext search testing.
"""

import re
import sys
import os
from urllib.request import urlopen
from html.parser import HTMLParser

# Add parent directory to path for ironbase import
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from ironbase import IronBase

MEK_URL = "https://mek.oszk.hu/00500/00598/00598.htm"
DB_PATH = os.path.join(os.path.dirname(__file__), "nepmesek.mlite")


class StoryParser(HTMLParser):
    """Parse HTML and extract stories with their paragraphs."""

    def __init__(self):
        super().__init__()
        self.stories = []
        self.current_story = None
        self.current_text = ""
        self.in_story = False
        self.in_title = False
        self.paragraph_buffer = []

    def handle_starttag(self, tag, attrs):
        attrs_dict = dict(attrs)

        # Story starts with <a name="N"> anchor
        if tag == "a" and "name" in attrs_dict:
            name = attrs_dict["name"]
            if name.isdigit():
                # Save previous story if exists
                if self.current_story and self.paragraph_buffer:
                    self.current_story["paragraphs"] = self.paragraph_buffer
                    self.stories.append(self.current_story)

                # Start new story
                self.current_story = {
                    "index": int(name),
                    "title": "",
                    "paragraphs": []
                }
                self.paragraph_buffer = []
                self.in_story = True
                self.in_title = True

        # Title is in <b> after anchor
        if tag == "b" and self.in_title:
            self.current_text = ""

        # Paragraph
        if tag == "p" and self.in_story:
            self.current_text = ""

    def handle_endtag(self, tag):
        if tag == "b" and self.in_title and self.current_story:
            self.current_story["title"] = self.current_text.strip()
            self.in_title = False

        if tag == "p" and self.in_story and self.current_text.strip():
            text = self.current_text.strip()
            # Clean up whitespace
            text = re.sub(r'\s+', ' ', text)
            if len(text) > 10:  # Skip very short paragraphs
                self.paragraph_buffer.append({
                    "num": len(self.paragraph_buffer) + 1,
                    "text": text
                })

    def handle_data(self, data):
        self.current_text += data

    def finalize(self):
        """Save the last story."""
        if self.current_story and self.paragraph_buffer:
            self.current_story["paragraphs"] = self.paragraph_buffer
            self.stories.append(self.current_story)


def fetch_and_parse():
    """Fetch HTML from MEK and parse stories."""
    print(f"Fetching from {MEK_URL}...")

    with urlopen(MEK_URL) as response:
        html = response.read().decode('iso-8859-2')  # Hungarian encoding

    print(f"Downloaded {len(html)} bytes")

    parser = StoryParser()
    parser.feed(html)
    parser.finalize()

    print(f"Parsed {len(parser.stories)} stories")
    return parser.stories


def create_nested_documents(stories):
    """Transform stories into nested document structure."""
    documents = []

    for story in stories:
        word_count = sum(len(p["text"].split()) for p in story["paragraphs"])
        char_count = sum(len(p["text"]) for p in story["paragraphs"])

        doc = {
            "title": story["title"],
            "index": story["index"],
            "metadata": {
                "source": "MEK - Magyar Elektronikus Konyvtar",
                "url": f"{MEK_URL}#{story['index']}",
                "word_count": word_count,
                "char_count": char_count,
                "paragraph_count": len(story["paragraphs"])
            },
            "paragraphs": story["paragraphs"],
            # Full text for fulltext search
            "full_text": " ".join(p["text"] for p in story["paragraphs"])
        }
        documents.append(doc)

    return documents


def populate_database(documents):
    """Insert documents into IronBase."""
    # Remove old database if exists
    if os.path.exists(DB_PATH):
        os.remove(DB_PATH)
        print(f"Removed old database: {DB_PATH}")

    db = IronBase(DB_PATH)
    print(f"Created database: {DB_PATH}")

    # Get collection
    coll = db.collection("mesek")

    # Insert all stories
    for doc in documents:
        coll.insert_one(doc)
        print(f"  Inserted: {doc['title']} ({doc['metadata']['word_count']} words, {doc['metadata']['paragraph_count']} paragraphs)")

    # Create fulltext index on full_text field with Hungarian language
    # Note: This requires MCP server or direct Rust API
    # For now, we just prepare the data

    total_words = sum(d["metadata"]["word_count"] for d in documents)
    total_paragraphs = sum(d["metadata"]["paragraph_count"] for d in documents)

    print(f"\n=== Summary ===")
    print(f"Stories: {len(documents)}")
    print(f"Total words: {total_words}")
    print(f"Total paragraphs: {total_paragraphs}")
    print(f"Database: {DB_PATH}")

    db.close()
    return DB_PATH


def main():
    stories = fetch_and_parse()
    documents = create_nested_documents(stories)
    db_path = populate_database(documents)

    print(f"\nDatabase ready at: {db_path}")
    print("Next: Create fulltext index with Hungarian language support")


if __name__ == "__main__":
    main()
