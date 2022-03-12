use std::{io::ErrorKind, str, time::Duration};

use hyper::{body, client::HttpConnector, http::request::Builder, Body, Client, Request};
use hyper_tls::HttpsConnector;
use mupdf::{
    pdf::{PdfDocument, PdfObject},
    TextPageOptions,
};
use tokio::{fs, time::sleep};

/// Returns a `Vec` of URLs with the colles' content as a PDF.
fn parse_week_list(s: &str) -> Vec<String> {
    let mut res = vec![];
    let mut remaining = s;
    loop {
        let link_marker = "<li><a href=\"";
        let link_start = match remaining.find(link_marker) {
            Some(val) => val + link_marker.len(),
            None => break,
        };
        let link_end = match remaining[link_start..].find('\"') {
            Some(val) => link_start + val,
            None => break,
        };
        res.push(remaining[link_start..link_end].to_owned());
        remaining = &remaining[(link_end + 1)..];
    }
    res
}

fn fake_browser(builder: Builder) -> Builder {
    builder
        .header(
            "Accept",
            "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8",
        )
        .header("Accept-Language", "en-US,en;q=0.5")
        .header("Cache-Control", "max-age=0")
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; rv:91.0) Gecko/20100101 Firefox/91.0",
        )
}

async fn fetch_week_list(
    client: &mut Client<HttpsConnector<HttpConnector>>,
) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    let url = "https://mp1.prepa-carnot.fr/programmes-de-colle/";
    let req = fake_browser(Request::get(url))
    .body(Body::empty())?;
    println!("Fetching colle program index at {}...", url);
    let resp = client.request(req).await?;
    if !resp.status().is_success() {
        panic!("colle program index response is not successful");
    }
    let body = body::to_bytes(resp.into_body()).await?;
    let body_str = str::from_utf8(&body)?;
    let ol = "<ol>";
    let ol_start = body_str.find(ol).expect("failed to find week list start");
    let ol_end = ol_start
        + ol.len()
        + body_str[ol_start + ol.len()..]
            .find("</ol>")
            .expect("failed to find week list end");
    let week_list_str = &body_str[ol_start + ol.len()..ol_end];
    Ok(parse_week_list(week_list_str))
}

fn extract_exercise_numbers(doc: &PdfDocument) -> Result<Vec<i32>, mupdf::Error> {
    let mut res = vec![];
    for page in doc.pages()? {
        let page = page?;
        let text_page = page.to_text_page(TextPageOptions::empty())?;
        for block in text_page.blocks() {
            for line in block.lines() {
                let string = line.chars().filter_map(|c| c.char()).collect::<String>();
                let ccinp = "CCINP ";
                let exercise_number_start = match string.find(ccinp) {
                    Some(val) => val + ccinp.len(),
                    None => continue,
                };
                let exercise_number_end = match string[exercise_number_start..].find(' ') {
                    Some(val) => exercise_number_start + val,
                    None => string.len(),
                };
                let exercise_number: i32 =
                    match string[exercise_number_start..exercise_number_end].parse() {
                        Ok(val) => val,
                        Err(_) => continue,
                    };
                if !res.contains(&exercise_number) {
                    res.push(exercise_number);
                }
            }
        }
    }
    Ok(res)
}

fn merge_pdf_document(dest: &mut PdfDocument, src: &PdfDocument) -> Result<(), mupdf::Error> {
    let page_count = src.page_count()?;
    let mut graft_map = dest.new_graft_map()?;
    for i in 0..page_count {
        let src_page = src.find_page(i)?;
        let mut dest_page = dest.new_dict()?;
        dest_page.dict_put("Type", PdfObject::new_name("Page")?)?;
        let to_copy = [
            "Contents",
            "Resources",
            "MediaBox",
            "CropBox",
            "BleedBox",
            "TrimBox",
            "ArtBox",
            "Rotate",
            "UserUnit",
        ];
        for name in to_copy {
            if let Some(src_obj) = src_page.get_dict_inheritable(name)? {
                let dest_obj = graft_map.graft_object(&src_obj)?;
                dest_page.dict_put(name, dest_obj)?;
            }
        }
        dest.add_object(&dest_page)?;
        dest.insert_page(dest.page_count()?, &dest_page)?;
    }
    Ok(())
}

async fn generate_fat_pdf(
    orig_url: &str,
    output_filename: &str,
    client: &mut Client<HttpsConnector<HttpConnector>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let req = fake_browser(Request::get(orig_url)).body(Body::empty())?;
    println!("Fetching colle content PDF at {}...", orig_url);
    let resp = client.request(req).await?;
    if !resp.status().is_success() {
        return Err(Box::new(std::io::Error::new(
            ErrorKind::Other,
            format!("failed to fetch colle content PDF at {}", orig_url),
        )));
    }
    let pdf = body::to_bytes(resp.into_body()).await?;
    let mut doc = match PdfDocument::from_bytes(&pdf) {
        Ok(val) => val,
        Err(err) => {
            return Err(Box::new(std::io::Error::new(
                ErrorKind::InvalidData,
                format!("failed to open colle content PDF: {}", err),
            )))
        }
    };
    let exercise_numbers = extract_exercise_numbers(&doc)?;
    println!("CCINP exercises: {:?}", exercise_numbers);
    if !exercise_numbers.is_empty() {
        let tmp = exercise_numbers
            .iter()
            .map(|n| n.to_string())
            .collect::<Vec<String>>()
            .join(",");
        let uri = format!("https://ccinp.mpsi1.fr/{}.pdf", tmp);
        println!("Fetching CCINP exercises PDF at {}...", uri);
        let resp = client.get(uri.parse()?).await?;
        if !resp.status().is_success() {
            return Err(Box::new(std::io::Error::new(
                ErrorKind::Other,
                format!("failed to fetch CCINP exercises PDF at {}", uri),
            )));
        }
        let exercises_pdf = body::to_bytes(resp.into_body()).await?;
        let exercises_doc = PdfDocument::from_bytes(&exercises_pdf)?;
        // Add the exercises at the end of the document.
        merge_pdf_document(&mut doc, &exercises_doc)?;
    }
    println!("Saving fat PDF to {}...", output_filename);
    doc.save(output_filename)?;
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let https = HttpsConnector::new();
    let mut client = Client::builder().build::<_, hyper::Body>(https);

    let week_list = fetch_week_list(&mut client).await?;
    for (i, pdf_url) in week_list.iter().enumerate() {
        let output_filename = format!("{}.pdf", i + 1);
        match fs::metadata(&output_filename).await {
            // The file already exists.
            Ok(_) => {
                println!(
                    "Skipping week {} because file {} already exists.",
                    output_filename,
                    i + 1
                );
                continue;
            }
            // The file does not exist, so generate it.
            Err(err) if err.kind() == ErrorKind::NotFound => {}
            Err(err) => panic!("failed to check metadata of {}: {}", output_filename, err),
        }
        println!("Waiting before sending a new request...");
        sleep(Duration::from_secs(3)).await;
        println!("Generating fat PDF for week {}...", i + 1);
        if let Err(err) = generate_fat_pdf(pdf_url, &output_filename, &mut client).await {
            eprintln!("Failed to generate fat PDF: {}", err);
        }
    }
    Ok(())
}
