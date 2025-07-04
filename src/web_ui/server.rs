use std::io::prelude::*;
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;

use crate::store::KG;
use crate::utils::{escape_html, linkify, schema_link, to_link};
use crate::web_ui::html_templates::{self, entity_page, index_page, query_page};
use crate::store::StoreError;
enum Page {
    Index,
    Explore(String, u32),
    Query(Option<String>, Option<String>),
    Entity(String),
    Error,
}

pub(crate) struct WebServer {
    dataset: Arc<KG>,
    port: u32,
}

impl WebServer {
    pub fn new(kg: KG, port: u32) -> WebServer {
        let kg = Arc::new(kg);
        WebServer { dataset: kg, port }
    }

    pub(crate) fn serve(&self) {
        let listener = TcpListener::bind(format!("127.0.0.1:{}", self.port)).unwrap();
        println!("Listening on http://127.0.0.1:{}", self.port);

        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let dataset_clone = self.dataset.clone();
                    thread::spawn(move || {
                        let server = WebServer {
                            dataset: dataset_clone,
                            port: 0,
                        };
                        server.handle_connection(stream);
                    });
                }
                Err(e) => {
                    eprintln!("Failed to accept connection: {}", e);
                }
            }
        }
    }

    fn handle_connection(&self, mut stream: TcpStream) {
        let mut buffer = [0; 1024];
        if stream.read(&mut buffer).is_err() {
            return;
        }

        let request = String::from_utf8_lossy(&buffer);
        let first_line = request.lines().next().unwrap_or("");

        let full_path = first_line
            .split_whitespace()
            .nth(1)
            .unwrap_or("/");

        let (route, query_string) = match full_path.split_once('?') {
            Some((r, q)) => (r, Some(q)),
            None => (full_path, None),
        };

        let (status_line, page) = match route {
            "/" => ("HTTP/1.1 200 OK", Page::Index),
            "/query" => {
              if let Some(qs) = query_string {
                // let q = percent_encoding::percent_decode_str(qs).decode_utf8().unwrap().to_string();
                 ("HTTP/1.1 200 OK", Page::Query(Self::extract_query_param(qs, "query"), Self::extract_query_param(qs, "mode")))
              } else {
                ("HTTP/1.1 200 OK", Page::Query(Self::extract_query_param("", "query"), None))
              }
            },
            "/explore" => {
                if let Some(qs) = query_string {
                    if let Some(id) = Self::extract_query_param(qs, "id") {
                        let page:u32 = match Self::extract_query_param(qs, "page").unwrap_or("1".to_string()).parse(){
                            Ok(num) => num,
                            Err(_) => 1,
                        };
                        ("HTTP/1.1 200 OK", Page::Explore(id, page))
                    } else {
                        ("HTTP/1.1 400 BAD REQUEST", Page::Error)
                    }
                } else {
                    ("HTTP/1.1 400 BAD REQUEST", Page::Error)
                }
            }
            route if route.starts_with("/entity/") => {
              let entity_name = &full_path["/entity/".len()..];
              ("HTTP/1.1 200 OK", Page::Entity(entity_name.to_string().replace("%3C", "<").replace("%3E", ">")))
          }
            "/entity" => ("HTTP/1.1 400 BAD REQUEST", Page::Entity("dfa".to_owned())),
            _ => ("HTTP/1.1 404 NOT FOUND", Page::Error),
        };

        let contents: String = match page {
            Page::Index => self.generate_index(),
            Page::Explore(id, page) => self.generate_explore(&id, page),
            Page::Query(Some(q), Some(mode)) => self.generate_query(&q, &mode),
            Page::Query(None, _) =>self.generate_query("", "query"),
            Page::Query(Some(q), None) => self.generate_query(&q, "query"),
            Page::Entity( uri) =>self.generate_entity(&uri),
            Page::Error => self.generate_error(),
        };

        let response = format!(
            "{status_line}\r\nContent-Length: {}\r\n\r\n{}",
            contents.len(),
            contents
        );

        let _ = stream.write_all(response.as_bytes());
    }

    fn extract_query_param(query: &str, key: &str) -> Option<String> {
        for pair in query.split('&') {
            let p  =percent_encoding::percent_decode_str(pair).decode_utf8().unwrap().to_string();
            let mut parts = p.splitn(2, '=');
            let k = parts.next()?;
            let v = parts.next()?;
            if k == key {
                return Some(v.replace("+", " ").replace("%20", " "));
            }
        }
        None
    }

    fn generate_index(&self) -> String {
        let q = r#"SELECT ?t (COUNT(?s) AS ?count)
    WHERE {
      ?s a ?t .
    }
    GROUP BY ?t
    ORDER BY DESC(?count)
    "#;
    
        let mut class_counts = vec![];
    
        let res = self.dataset.query(q);
        match res {
            Ok(result) => {
                for r in result {
                    let class = r.get("t").unwrap().to_string();
                    let cnt = match r.get("count").unwrap() {
                        oxigraph::model::Term::Literal(literal) => literal.value().parse::<u32>().unwrap(),
                        _ => 0,
                    };
                    class_counts.push((class, cnt));
                }
            }
            Err(_) => panic!("SPARQL query failed"),
        }
    
        index_page(&self.dataset.dataset, &class_counts)
    }
    
  
    fn generate_explore(&self, id: &str, page_num: u32) -> String {
        let objs = self.dataset.get_objects(
            &id.replace("%3C", "<").replace("%3E", ">"), 50, (page_num-1)*50);
        let mut page = String::new();
        
        for o in objs{
            page +=&self.dataset.get_info(o).html_rep();
        }
        let mut navigation = String::new();

navigation += r#"<div class="d-flex justify-content-between gap-2 mt-4">"#;

// Previous button
if page_num > 1 {
    navigation += &format!(
        r#"<a class="btn btn-outline-primary" href="/explore?id={id}&page={}">Previous</a>"#,
        page_num - 1
    );
} else {
    navigation += r#"<button class="btn btn-outline-secondary" disabled>Previous</button>"#;
}
navigation+=&format!("<p>Page {page_num}</p>");
// Next button
navigation += &format!(
    r#"<a class="btn btn-outline-primary" href="/explore?id={id}&page={}">Next</a>"#,
    page_num + 1
);

navigation += "</div>";
        html_templates::explore_page(&page, &navigation)
    }

    fn generate_query(&self, q: &str, mode: &str) -> String {
        let mut table_data = vec![];
        let mut headers = vec![];
        let mut message = "Query successfully executed".to_string();
        let mut messageType = "success";

        if !q.is_empty() {
            match mode {
                "query" => {
                    let query_result = self.dataset.query(q);
                    match query_result{
                        Ok(res) => {
                            if !res.is_empty() {
                                headers = res[0].variables()
                                    .into_iter()
                                    .map(|var| var.clone().into_string())
                                    .collect::<Vec<String>>();
                    
                                for r in res {
                                    let row = r.values().into_iter().map(|v| {
                                        match v {
                                            Some(t) => {
                                                let val = t.to_string();
                                                if val.starts_with('<') && val.ends_with('>') {
                                                    let inner = &val[1..val.len() - 1];
                                                    format!("<{}>", inner) // Keep brackets for now, interpreted in JS
                                                } else {
                                                    val
                                                }
                                            }
                                            None => "None".to_owned(),
                                        }
                                    }).collect::<Vec<String>>();
                                    table_data.push(row);
                                }
                            }
                        }
                        Err(StoreError::EvaluationError (ee)) =>{
                            message  = ee;
                            messageType = "danger";
                        },
                        Err(StoreError::UnsupportedError) => {
                            message = "The query is not yet supported".to_string();
                            messageType = "danger";

                        }
                    }
                },
                "update" => {
                    let query_result = self.dataset.update(q);
                    match query_result{
                        Ok(()) => (),
                        Err(StoreError::EvaluationError(ee)) => {
                            message=  ee;
                            messageType = "danger";
                        }
                        _ => ()
                    }
                }
                _ => ()

            }
            
        }

        let result_rows = table_data.len();

        // JavaScript-safe string
        let mut table_rows_js_array = String::new();
        for row in table_data {
            let cells: Vec<String> = row.into_iter().map(|cell| {
                let escaped = cell.replace('\\', "\\\\").replace('"', "\\\"");
                format!(r#""{}""#, escaped)
            }).collect();
            table_rows_js_array += &format!("[{}],", cells.join(","));
        }
        let table_headers_js_array = headers.iter()
            .map(|h| format!(r#""{}""#, h))
            .collect::<Vec<_>>()
            .join(",");
        let message_box = if(q.is_empty()) {""} else {
            &format!("<div class=\"alert alert-{}\" role=\"alert\"> {} </div>", messageType, message)
        };
        query_page(result_rows, &table_rows_js_array, &table_headers_js_array,message_box )
    }

    fn generate_entity(&self, entity: &str) -> String{
      let itm = self.dataset.details(entity);
      //Table 1 generation
      let table_1_query= format!(r#"
      SELECT ?pred ?obj WHERE {{
        {entity} ?pred ?obj .
      }}
      "#);
      let table_1_data = self.dataset.query(&table_1_query).unwrap_or(vec![]);
      let mut table_1 = String::new();
      for row in table_1_data{

      table_1+=&format!("<tr>
          <td>{}</td>
          <td>{}</td>
        </tr>", escape_html(row.get("pred").unwrap().to_string())  ,  to_link(escape_html(row.get("obj").unwrap().to_string())) );

      }
      //Table 2 generation
      let table_2_query= format!(r#"
      SELECT ?sub ?pred WHERE {{
        ?sub ?pred {entity} .
      }}
      "#);
      let table_2_data = self.dataset.query(&table_2_query).unwrap_or(vec![]);
      let mut table_2 = String::new();
      for row in table_2_data{
        table_2+=&format!("<tr>
          <td>{}</td>
          <td>{}</td>
        </tr>", to_link(escape_html(row.get("sub").unwrap().to_string()) ) ,  escape_html(row.get("pred").unwrap().to_string()) );

      }


      let mut entity_types = String::new();
      for tp in itm.entity_types {
        entity_types += &schema_link(escape_html( tp.to_string()))
      }
      
      let name = &itm.name.unwrap_or("No name found".to_string());
      
      let img = if itm.images.is_empty() {
        String::new()
      } else {
        format!("<img src=\"{}\" alt=\"{}\"  class=\"object-fit-cover d-block mx-auto\" style=\"
          height: 200px;
        \" />", itm.images.get(0).unwrap(), name)
      };

      entity_page(&linkify(&entity),&name, &itm.description.unwrap_or("No description found".to_string()),  &entity_types, &img, &table_1, &table_2)
    }

    fn generate_error(&self) -> String {
        "<html><body><h1>404 - Page Not Found</h1></body></html>".to_string()
    }
}
