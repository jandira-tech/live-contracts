"""Tests for scanned-exhibit image capture (hosted in the HF dataset)."""
from sec_listener import images
from sec_listener.api import clean_excerpt


def test_image_filenames_in_order_deduped():
    md = 'Exhibit 10.3 (ex10-3_001.jpg) (ex10-3_002.jpg) (ex10-3_001.jpg) (logo.PNG "x")'
    assert images.image_filenames(md) == ["ex10-3_001.jpg", "ex10-3_002.jpg", "logo.PNG"]
    assert images.image_filenames("") == []
    assert images.image_filenames(None) == []


def test_is_image_only_true_when_body_is_just_images():
    # After clean_excerpt strips image refs + labels, nothing readable remains.
    md = "Exhibit 10.3 (ex10-3_001.jpg) (ex10-3_002.jpg) (ex10-3_003.jpg)"
    assert images.is_image_only(md, clean_excerpt_fn=clean_excerpt) is True


def test_is_image_only_false_when_there_is_text():
    md = '(exhibit101.jpg "slide1") Exhibit 10.1 FACILITY AGREEMENT dated 19 May 2026 between parties'
    assert images.is_image_only(md, clean_excerpt_fn=clean_excerpt) is False
    # No images at all -> not image-only either.
    assert images.is_image_only("Just some contract text.", clean_excerpt_fn=clean_excerpt) is False


def test_dataset_path_and_public_url():
    p = images.dataset_image_path("0001679273-26-000018", "ex10-1_001.jpg")
    assert p == "images/0001679273-26-000018/ex10-1_001.jpg"
    url = images.public_url("arthrod/sec-ex10-exhibits", p)
    assert url == "https://huggingface.co/datasets/arthrod/sec-ex10-exhibits/resolve/main/images/0001679273-26-000018/ex10-1_001.jpg"


def test_capture_images_uploads_and_returns_urls():
    uploaded = []

    def fake_fetcher(accession, cik):
        return [("ex10-1_001.jpg", b"\xff\xd8jpegbytes"), ("ex10-1_002.jpg", b"\xff\xd8more")]

    def fake_uploader(data, path, repo, token):
        uploaded.append((path, len(data), repo, token))

    urls = images.capture_images(
        "0001679273-26-000018", "1679273",
        token="tok", repo="arthrod/sec-ex10-exhibits",
        fetcher=fake_fetcher, uploader=fake_uploader,
    )
    assert urls == [
        "https://huggingface.co/datasets/arthrod/sec-ex10-exhibits/resolve/main/images/0001679273-26-000018/ex10-1_001.jpg",
        "https://huggingface.co/datasets/arthrod/sec-ex10-exhibits/resolve/main/images/0001679273-26-000018/ex10-1_002.jpg",
    ]
    assert uploaded[0] == ("images/0001679273-26-000018/ex10-1_001.jpg", 11, "arthrod/sec-ex10-exhibits", "tok")


def test_capture_images_noop_without_token():
    called = []
    urls = images.capture_images("a", "1", token="", repo="r",
                                 fetcher=lambda *a: called.append("f") or [],
                                 uploader=lambda *a: called.append("u"))
    assert urls == [] and called == []
