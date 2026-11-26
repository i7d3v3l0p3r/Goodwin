import json
from pathlib import Path
from tempfile import TemporaryDirectory
import unittest
import wave

from wakeword_manifest import (
    RATE, check_fingerprint, check_heldout, load_corpus, training_fingerprint, verified_pcm,
)


def clip(root, name, split, group, sample, label=0):
    path = f"{name}.wav"
    with wave.open(str(root / path), "wb") as audio:
        audio.setparams((1, 2, RATE, 0, "NONE", "not compressed"))
        audio.writeframes(sample.to_bytes(2, "little", signed=True) * 160)
    return dict(path=path, split=split, group=group, label=label, duration_s=0.01,
                category="positive" if label else "noise", phrase_onset_s=0, phrase_offset_s=0.005)


def save_manifest(root, clips):
    (root / "manifest.jsonl").write_text("".join(json.dumps(row) + "\n" for row in clips), encoding="utf-8")


class WakeCorpusTests(unittest.TestCase):
    def test_pcm_duplicates_keep_a_stable_copy_in_the_earliest_split(self):
        with TemporaryDirectory() as work:
            root = Path(work)
            rows = [
                clip(root, "test", "test", "unseen", 5),
                clip(root, "second", "train", "seen", 5),
                clip(root, "first", "train", "seen", 5),
                clip(root, "conflict-a", "train", "seen", 6),
                clip(root, "conflict-b", "validation", "validation", 6, label=1),
            ]
            save_manifest(root, rows)
            corpus = load_corpus(root)
            save_manifest(root, reversed(rows))
            self.assertEqual(load_corpus(root).splits, corpus.splits)
            self.assertEqual([r.meta["path"] for r in corpus.splits["train"]], ["first.wav"])
            self.assertEqual(corpus.splits["validation"], [])
            self.assertEqual(corpus.splits["test"], [])
            self.assertEqual(len(corpus.duplicates), 4)

    def test_source_group_cannot_span_splits_with_different_pcm(self):
        with TemporaryDirectory() as work:
            root = Path(work)
            save_manifest(root, [clip(root, "train", "train", "source", 1),
                                 clip(root, "test", "test", "source", 2)])
            with self.assertRaisesRegex(ValueError, "group spans multiple splits"):
                load_corpus(root)

    def test_invalid_duration_or_path_is_rejected(self):
        with TemporaryDirectory() as work:
            root = Path(work)
            row = clip(root, "clip", "train", "source", 1)
            save_manifest(root, [{**row, "duration_s": 0.5}])
            with self.assertRaisesRegex(ValueError, "Invalid WAV duration"):
                load_corpus(root)
            save_manifest(root, [{**row, "path": "../clip.wav"}])
            with self.assertRaisesRegex(ValueError, "relative to the corpus"):
                load_corpus(root)

    def test_external_test_rejects_both_source_and_pcm_overlap(self):
        with TemporaryDirectory() as work, TemporaryDirectory() as other:
            root, heldout = Path(work), Path(other)
            save_manifest(root, [clip(root, "train", "train", "seen", 1),
                                 clip(root, "validation", "validation", "validation", 2)])
            corpus = load_corpus(root)
            check_heldout(corpus, corpus)
            unrelated = clip(heldout, "unrelated", "test", "new", 3)
            save_manifest(heldout, [unrelated])
            check_heldout(corpus, load_corpus(heldout))
            save_manifest(heldout, [unrelated, clip(heldout, "source", "test", "seen", 4),
                                    clip(heldout, "copy", "test", "other", 2)])
            with self.assertRaisesRegex(ValueError, "1 groups, 1 PCM hashes"):
                check_heldout(corpus, load_corpus(heldout))

    def test_checkpoint_fingerprint_covers_training_pcm_but_not_unseen_pcm(self):
        with TemporaryDirectory() as work:
            root = Path(work)
            train = clip(root, "train", "train", "seen", 1)
            test = clip(root, "test", "test", "unseen", 2)
            save_manifest(root, [train, test])
            corpus = load_corpus(root)
            fingerprint = training_fingerprint(corpus)
            clip(root, "test", "test", "unseen", 3)
            self.assertTrue(check_fingerprint(fingerprint, training_fingerprint(load_corpus(root)), "Checkpoint"))
            clip(root, "train", "train", "seen", 4)
            with self.assertRaisesRegex(ValueError, "WAV changed"):
                verified_pcm(corpus, corpus.splits["train"][0])
            current = training_fingerprint(load_corpus(root))
            with self.assertRaisesRegex(ValueError, "different training or validation data"):
                check_fingerprint(fingerprint, current, "Checkpoint")
            with self.assertWarnsRegex(UserWarning, "predates corpus fingerprints"):
                self.assertFalse(check_fingerprint(None, current, "Checkpoint"))


if __name__ == "__main__":
    unittest.main()
