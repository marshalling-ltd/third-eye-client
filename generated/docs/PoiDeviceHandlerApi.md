# \PoiDeviceHandlerApi

All URIs are relative to *http://127.0.0.1:8070*

Method | HTTP request | Description
------------- | ------------- | -------------
[**create_poi_device_handler**](PoiDeviceHandlerApi.md#create_poi_device_handler) | **POST** /api/v1/poi-devices | 
[**delete_poi_device_handler**](PoiDeviceHandlerApi.md#delete_poi_device_handler) | **DELETE** /api/v1/poi-devices/{id} | 
[**get_poi_device_handler**](PoiDeviceHandlerApi.md#get_poi_device_handler) | **GET** /api/v1/poi-devices/{id} | 
[**list_poi_device_handler**](PoiDeviceHandlerApi.md#list_poi_device_handler) | **GET** /api/v1/poi-devices | 



## create_poi_device_handler

> models::PoiDeviceExtendedModel create_poi_device_handler(create_poi_device_schema)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**create_poi_device_schema** | [**CreatePoiDeviceSchema**](CreatePoiDeviceSchema.md) | Create point of interest device | [required] |

### Return type

[**models::PoiDeviceExtendedModel**](PoiDeviceExtendedModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: application/json
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## delete_poi_device_handler

> delete_poi_device_handler(id)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**id** | **uuid::Uuid** | id for deleting PoI device | [required] |

### Return type

 (empty response body)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## get_poi_device_handler

> models::PoiDeviceExtendedModel get_poi_device_handler(id)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**id** | **uuid::Uuid** | id for fetching PoI device | [required] |

### Return type

[**models::PoiDeviceExtendedModel**](PoiDeviceExtendedModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## list_poi_device_handler

> models::PagedResponsePoiDeviceExtendedModel list_poi_device_handler(page, limit, order_by, desc, poi_id, device_id, device_name, storage_secret_id)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**page** | Option<**i32**> | page number |  |
**limit** | Option<**i32**> | page size |  |
**order_by** | Option<**String**> | list order by |  |
**desc** | Option<**bool**> | should use descending instead |  |
**poi_id** | Option<**uuid::Uuid**> | PoI id |  |
**device_id** | Option<**uuid::Uuid**> | device id |  |
**device_name** | Option<**String**> | device name |  |
**storage_secret_id** | Option<**uuid::Uuid**> | storage secret id |  |

### Return type

[**models::PagedResponsePoiDeviceExtendedModel**](PagedResponse_PoiDeviceExtendedModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)

