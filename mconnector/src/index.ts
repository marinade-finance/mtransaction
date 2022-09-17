import { credentials, loadPackageDefinition, ServiceClientConstructor } from "@grpc/grpc-js";
import { loadSync } from "@grpc/proto-loader";
import path = require("path");

var PROTO_PATH = path.join(__dirname, '../..', 'proto', 'mtransaction.proto');

var packageDefinition = loadSync(
    PROTO_PATH,
    {keepCase: true,
     longs: String,
     enums: String,
     defaults: true,
     oneofs: true
    });
var grpcObj = loadPackageDefinition(packageDefinition);
const validatorPackage = grpcObj.validator;
const constructor = validatorPackage["MTransaction"] as ServiceClientConstructor;

function main() {
  var target = 'http://192.168.200.176:50051/';
  console.log(validatorPackage)
  var client = new constructor(target, credentials.createInsecure());
  client.EchoRequest({}, function(err: unknown, response: unknown) {
    if (err) throw err;
    console.log('Greeting:', response);
    client.close();
  });
}

main();